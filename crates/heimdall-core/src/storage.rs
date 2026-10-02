//! The vault capability and every filesystem operation Heimdall performs.
//!
//! The vault is opened once as a `cap-std` directory capability; all
//! descendants resolve beneath it. Containment is enforced by the capability
//! itself, not by comparing path strings (SPEC §14), so `..` components,
//! absolute children, and symlinks pointing out of the vault fail at the
//! syscall boundary rather than being filtered by hand.
//!
//! The things deliberately kept outside that capability are the write lock and
//! the vault's note locks. Both are Heimdall's own state rather than vault
//! content, and a vault is a folder of the user's Markdown — nothing of ours
//! belongs in it.

use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use camino::{Utf8Path, Utf8PathBuf};
use cap_std::ambient_authority;
use cap_std::fs::{Dir, OpenOptions};
use fs4::fs_std::FileExt;
use time::OffsetDateTime;

use crate::appdata;
use crate::errors::{Error, Result};
use crate::paths::RelPath;
use crate::revisions::Revision;
use crate::timestamps;

/// Distinguishes concurrent temp files written by one process.
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// How long to wait for a contended lock before reporting that the holder is
/// stuck. Every holder does bounded work, so this is far above normal waiting.
const LOCK_WAIT_LIMIT: Duration = Duration::from_secs(10);

/// How often to retry a contended lock while waiting.
const LOCK_POLL_INTERVAL: Duration = Duration::from_millis(2);

/// Size and modification time for one vault path.
#[derive(Debug, Clone)]
pub struct FileMeta {
    pub size_bytes: u64,
    pub modified_at: OffsetDateTime,
}

/// One resolved child of a directory.
#[derive(Debug, Clone)]
pub struct DirChild {
    pub name: String,
    pub is_dir: bool,
    pub meta: FileMeta,
}

/// An opened vault: the security and content boundary for a session.
#[derive(Debug)]
pub struct Vault {
    dir: Dir,
    /// The canonical location of this vault. Canonical rather than as-typed
    /// because it is the key two processes must agree on to share a lock.
    root: Utf8PathBuf,
    /// Heimdall's per-user application data, outside the vault itself: the
    /// write lock and the note-lock rules both live beneath it.
    data_dir: Utf8PathBuf,
}

impl Vault {
    /// Open an existing directory as a vault capability.
    pub fn open(root: &Utf8Path) -> Result<Self> {
        Self::open_inner(root, appdata::data_dir()?)
    }

    /// Open a vault whose application data lives somewhere chosen by the caller.
    ///
    /// Only tests need this. They must not write into the developer's real
    /// application-data directory, and an environment variable would be racy
    /// across the parallel threads the concurrency tests already use.
    pub fn open_with_data_dir(root: &Utf8Path, data_dir: &Utf8Path) -> Result<Self> {
        Self::open_inner(root, data_dir.to_owned())
    }

    fn open_inner(root: &Utf8Path, data_dir: Utf8PathBuf) -> Result<Self> {
        let dir = Dir::open_ambient_dir(root.as_std_path(), ambient_authority()).map_err(|err| {
            match err.kind() {
                std::io::ErrorKind::NotFound => {
                    // Tagged, so the desktop can tell "this vault's folder is
                    // gone" from a note that is not there.
                    Error::not_found("vault directory does not exist")
                        .with_detail("reason", "vault_missing")
                }
                std::io::ErrorKind::NotADirectory => {
                    Error::invalid_input("vault path is not a directory")
                }
                std::io::ErrorKind::PermissionDenied => Error::os_permission("open vault", &err),
                _ => Error::from_io("open vault", &err),
            }
        })?;

        // The canonical path, not the one that was typed. Two processes naming
        // one vault differently — through a symlink, or by a relative path —
        // must derive the same lock, or they would each take a lock of their
        // own and exclude nothing. This grants no authority the line above did
        // not already use: the same ambient path was just resolved to open the
        // capability.
        let root = canonicalize(root)?;

        Ok(Self {
            dir,
            root,
            data_dir,
        })
    }

    /// The canonical path of this vault.
    pub fn root(&self) -> &Utf8Path {
        &self.root
    }

    /// The vault's folder name, for a person reading a prompt or a message.
    pub fn name(&self) -> &str {
        self.root.file_name().unwrap_or("vault")
    }

    /// Every Markdown note at or beneath `dir`, spelled as stored.
    ///
    /// For flagging notes, so it never follows a symlink — a link inside the
    /// vault names a note this walk reaches by its real path anyway, and one
    /// pointing out of it names a file that is not the vault's to protect —
    /// and never enters a hidden folder, which belongs to another tool.
    pub fn notes_beneath(&self, dir: &RelPath) -> Vec<RelPath> {
        let mut notes = Vec::new();
        let mut pending = vec![dir.clone()];
        while let Some(current) = pending.pop() {
            let Ok(entries) = self.dir.read_dir(resolve(&current)) else {
                continue;
            };
            for entry in entries.flatten() {
                let Ok(name) = entry.file_name().into_string() else {
                    continue;
                };
                if !crate::paths::is_listable(&name) {
                    continue;
                }
                let Ok(kind) = entry.file_type() else { continue };
                let path = current.join(&name);
                if kind.is_dir() {
                    pending.push(path);
                } else if kind.is_file() && path.is_markdown() {
                    notes.push(path);
                }
            }
        }
        notes.sort();
        notes
    }

    /// Where this vault's application data lives.
    pub fn data_dir(&self) -> &Utf8Path {
        &self.data_dir
    }

    pub fn exists(&self, path: &RelPath) -> bool {
        self.dir.exists(resolve(path))
    }

    /// Whether a directory entry of this name exists, without following it.
    ///
    /// [`Vault::exists`] resolves symlinks, so it answers "no" for a link
    /// pointing out of the vault — the same answer it gives for a name that was
    /// never there. Distinguishing the two lets a caller be told its path
    /// escapes the vault rather than that it does not exist.
    pub fn entry_exists(&self, path: &RelPath) -> bool {
        self.dir.symlink_metadata(resolve(path)).is_ok()
    }

    /// The path as it is spelled on disk, with every symlink inside the vault
    /// resolved.
    ///
    /// A lock is a rule about a file, but a caller names a file with a string,
    /// and on macOS many strings name the same file: APFS ignores case and
    /// Unicode normalization, so `Projects/Plan.md` opens `projects/plan.md`,
    /// and a symlinked folder inside the vault is a second name for the folder
    /// it points at. Comparing rules against only what the caller typed would
    /// let any of those spellings walk past a lock. `realpath(3)` reports each
    /// component as it is stored, which is the one spelling every caller can be
    /// held to.
    ///
    /// The part of the path that does not exist yet — a note about to be
    /// created — is appended as typed beneath the deepest part that does. A
    /// path that resolves outside the vault is `PATH_OUTSIDE_VAULT`, the same
    /// answer the capability gives when it is used.
    ///
    /// This only computes a name to check rules against. Nothing is opened or
    /// changed through the ambient path; every access still goes through the
    /// capability.
    pub fn real_path(&self, path: &RelPath) -> Result<RelPath> {
        let mut existing = path.clone();
        let mut tail: Vec<String> = Vec::new();
        loop {
            if existing.is_root() {
                break;
            }
            match std::fs::canonicalize(self.root.join(existing.as_str()).as_std_path()) {
                Ok(resolved) => {
                    let Ok(resolved) = Utf8PathBuf::from_path_buf(resolved) else {
                        return Err(Error::invalid_input(format!(
                            "\"{path}\" does not resolve to a UTF-8 path"
                        ))
                        .with_detail("path", path.as_str()));
                    };
                    let Ok(relative) = resolved.strip_prefix(&self.root) else {
                        return Err(Error::path_outside_vault(format!(
                            "\"{path}\" resolves outside the vault"
                        ))
                        .with_detail("path", path.as_str()));
                    };
                    let mut real = RelPath::parse(relative.as_str())?;
                    for name in tail.iter().rev() {
                        real = real.join(name);
                    }
                    return Ok(real);
                }
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                    let name = existing.file_name().unwrap_or_default().to_string();
                    tail.push(name);
                    existing = existing.parent();
                }
                Err(err) => return Err(Error::from_io_path(&format!("resolve {path}"), &err)),
            }
        }
        // Nothing of it exists; the vault root is its own real spelling.
        let mut real = RelPath::root();
        for name in tail.iter().rev() {
            real = real.join(name);
        }
        Ok(real)
    }

    /// Whether a file carries the filesystem's immutable flag — the one a
    /// Heimdall lock sets on a note, and the one Finder's "Locked" sets.
    ///
    /// `false` for a folder, for anything that cannot be opened, and on
    /// platforms with no such flag.
    pub fn is_immutable(&self, path: &RelPath) -> bool {
        osflags::is_immutable(&self.dir, resolve(path))
    }

    /// Set or clear the immutable flag on one file. Returns whether the flag
    /// changed. Only `lock` and `unlock` call this (SPEC §6): Heimdall never
    /// lifts a flag on its own initiative.
    pub fn set_immutable(&self, path: &RelPath, immutable: bool) -> Result<bool> {
        osflags::set_immutable(&self.dir, resolve(path), immutable)
            .map_err(|err| Error::from_io_path(&format!("protect {path}"), &err))
    }

    pub fn is_file(&self, path: &RelPath) -> bool {
        self.dir.is_file(resolve(path))
    }

    pub fn is_dir(&self, path: &RelPath) -> bool {
        self.dir.is_dir(resolve(path))
    }

    /// Size and modification time, following symlinks through the capability.
    pub fn meta(&self, path: &RelPath) -> Result<FileMeta> {
        let meta = self
            .dir
            .metadata(resolve(path))
            .map_err(|err| Error::from_io_path(&format!("stat {path}"), &err))?;
        let modified_at = meta
            .modified()
            .map_err(|err| Error::from_io(&format!("stat {path}"), &err))
            .and_then(timestamps::from_cap)
            // A filesystem without modification times is unusual but not a
            // reason to fail a listing; fall back to the epoch.
            .unwrap_or(OffsetDateTime::UNIX_EPOCH);
        Ok(FileMeta {
            size_bytes: meta.len(),
            modified_at,
        })
    }

    /// Read a file's exact bytes.
    pub fn read(&self, path: &RelPath) -> Result<Vec<u8>> {
        if self.dir.is_dir(resolve(path)) {
            return Err(Error::invalid_input(format!(
                "\"{path}\" is a directory; select a file to read"
            ))
            .with_detail("path", path.as_str()));
        }
        self.dir
            .read(resolve(path))
            .map_err(|err| Error::from_io_path(&format!("read {path}"), &err))
    }

    /// Directory children with resolved metadata, sorted by name.
    ///
    /// Entries whose metadata cannot be resolved are skipped rather than
    /// failing the listing: that is exactly how a symlink pointing outside the
    /// vault presents itself, and a listing must never expose one.
    pub fn children(&self, path: &RelPath) -> Result<Vec<DirChild>> {
        let entries = self
            .dir
            .read_dir(resolve(path))
            .map_err(|err| Error::from_io_path(&format!("list {path}"), &err))?;

        let mut children = Vec::new();
        for entry in entries {
            let Ok(entry) = entry else { continue };
            let Ok(name) = entry.file_name().into_string() else {
                // A non-UTF-8 filename cannot be addressed by the JSON contract.
                continue;
            };
            let child = path.join(&name);
            let Ok(meta) = self.meta(&child) else { continue };
            let is_dir = self.dir.is_dir(resolve(&child));
            children.push(DirChild { name, is_dir, meta });
        }
        children.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(children)
    }

    /// Child names only, sorted, without resolving any metadata.
    ///
    /// A walk that can reject a name outright — hidden, junk, not Markdown,
    /// already behind a cursor — should not pay for a `stat` first.
    pub fn child_names(&self, path: &RelPath) -> Result<Vec<String>> {
        let entries = self
            .dir
            .read_dir(resolve(path))
            .map_err(|err| Error::from_io_path(&format!("list {path}"), &err))?;

        let mut names: Vec<String> = entries
            .flatten()
            .filter_map(|entry| entry.file_name().into_string().ok())
            .collect();
        names.sort();
        Ok(names)
    }

    /// Resolve one child, or `None` if it cannot be resolved inside the vault.
    ///
    /// `None` is the normal answer for a symlink pointing out of the vault and
    /// for an entry deleted between listing and resolution; neither should fail
    /// a listing, and neither may be exposed.
    pub fn resolve_child(&self, path: &RelPath, name: &str) -> Option<DirChild> {
        let child = path.join(name);
        let meta = self.meta(&child).ok()?;
        Some(DirChild {
            name: name.to_string(),
            is_dir: self.dir.is_dir(resolve(&child)),
            meta,
        })
    }

    /// Create a directory and any missing parents. Existing paths are left alone.
    ///
    /// Returns the directories actually created, innermost last, so a failed
    /// scaffold can roll back exactly what it made.
    pub fn create_dir_all(&self, path: &RelPath) -> Result<Vec<RelPath>> {
        let mut created = Vec::new();
        let mut current = RelPath::root();
        for component in path.components() {
            current = current.join(component);
            if self.dir.is_dir(current.as_str()) {
                continue;
            }
            self.dir
                .create_dir(current.as_str())
                .map_err(|err| Error::from_io_path(&format!("create {current}/"), &err))?;
            created.push(current.clone());
        }
        Ok(created)
    }

    /// Remove a directory only if it is empty. Used for scaffold rollback.
    pub fn remove_dir_if_empty(&self, path: &RelPath) {
        let _ = self.dir.remove_dir(path.as_str());
    }

    /// Move a file or directory to another location inside the vault, refusing
    /// to replace anything already at the destination.
    ///
    /// POSIX `rename` silently replaces the destination, which SPEC §14 forbids.
    /// `cap-std` exposes no `renameat2(RENAME_NOREPLACE)`, so the check and the
    /// rename must both happen under the destination's file lock — the same
    /// discipline `write` uses around revision comparison. Callers are
    /// responsible for holding that lock.
    ///
    /// Being honest about what this does and does not guarantee: it serializes
    /// Heimdall processes against each other, not against another editor writing
    /// into the same vault. That boundary is already accepted — SPEC §8 says
    /// listings are weakly consistent while files change.
    pub fn rename_no_replace(&self, from: &RelPath, to: &RelPath) -> Result<()> {
        if self.entry_exists(to) {
            return Err(Error::already_exists(format!("\"{to}\" already exists"))
                .with_detail("path", to.as_str()));
        }
        self.dir
            .rename(from.as_str(), &self.dir, to.as_str())
            .map_err(|err| Error::from_io_path(&format!("move {from} to {to}"), &err))?;
        let from_parent = from.parent();
        let to_parent = to.parent();
        self.sync_dir(&from_parent);
        if to_parent.as_str() != from_parent.as_str() {
            self.sync_dir(&to_parent);
        }
        Ok(())
    }

    /// Write a complete file, replacing any existing content atomically.
    ///
    /// Content lands in a temporary sibling that is flushed to disk before the
    /// rename, so a reader either sees the previous file or the new one — never
    /// a partial write. Returns the revision of the bytes as stored.
    pub fn atomic_write(&self, path: &RelPath, bytes: &[u8]) -> Result<Revision> {
        let name = path
            .file_name()
            .ok_or_else(|| Error::internal("write target has no filename"))?;
        let parent = path.parent();
        let temp = parent.join(&format!(
            ".{name}.tmp-{}-{}",
            std::process::id(),
            TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let guard = TempGuard {
            vault: self,
            path: Some(temp.clone()),
        };

        {
            let mut options = OpenOptions::new();
            options.write(true).create(true).truncate(true);
            let mut file = self
                .dir
                .open_with(temp.as_str(), &options)
                .map_err(|err| Error::from_io_path(&format!("create temp for {path}"), &err))?;
            file.write_all(bytes)
                .map_err(|err| Error::from_io(&format!("write {path}"), &err))?;
            file.sync_all()
                .map_err(|err| Error::from_io(&format!("flush {path}"), &err))?;
        }

        self.dir
            .rename(temp.as_str(), &self.dir, path.as_str())
            .map_err(|err| Error::from_io_path(&format!("replace {path}"), &err))?;
        guard.disarm();
        self.sync_dir(&parent);

        Ok(Revision::of_bytes(bytes))
    }

    /// Create a file that must not already exist.
    ///
    /// `O_EXCL` is the create-only primitive: unlike write-then-rename it can
    /// never replace an existing file, which is what entries require (SPEC §10).
    /// `AlreadyExists` is returned to the caller so it can pick the next
    /// collision suffix.
    pub fn create_new(&self, path: &RelPath, bytes: &[u8]) -> Result<Revision> {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        let mut file = self
            .dir
            .open_with(path.as_str(), &options)
            .map_err(|err| Error::from_io_path(&format!("create {path}"), &err))?;
        file.write_all(bytes)
            .map_err(|err| Error::from_io(&format!("write {path}"), &err))?;
        file.sync_all()
            .map_err(|err| Error::from_io(&format!("flush {path}"), &err))?;
        self.sync_dir(&path.parent());
        Ok(Revision::of_bytes(bytes))
    }

    /// Run `body` holding this vault's cross-process write lock.
    ///
    /// Optimistic concurrency only holds if comparing a revision and replacing
    /// the file are one indivisible step. Without that, two writers both read
    /// revision A, both find it current, and both write — and one person's edit
    /// is gone with no error at all.
    ///
    /// The lock is one file per vault, and it lives outside the vault (see
    /// [`Vault::lock_path`]). It cannot live on the target: `atomic_write`
    /// renames a new inode over it, so each writer would end up holding a lock
    /// on a different file. It could live on a sidecar beside the target, and
    /// used to, but a sidecar can never be safely removed — unlinking one that
    /// another process is about to open leaves the two locking different inodes
    /// — so every note ever written left a `.lock` file behind in the user's
    /// vault. One lock per vault is coarser than per-file and therefore strictly
    /// stronger, and a holder only hashes some bytes and renames a temp file, so
    /// serialising a vault's writes costs nothing at the rate they arrive.
    ///
    /// `path` does not select the lock; it names what is being written, so a
    /// timeout can say which operation was waiting.
    ///
    /// **Never call a locking operation from inside `body`.** With one lock per
    /// vault that is a self-deadlock, not merely redundant: each acquire opens a
    /// fresh descriptor, and `flock` on a second open file description blocks
    /// even within one process.
    ///
    /// Waiting for the lock is bounded (SPEC §12). A blocking acquire would
    /// hang for as long as some other process — possibly one that is wedged, or
    /// stopped under a debugger — chooses to hold it, and a server cannot offer
    /// a bounded operation duration on top of an unbounded wait.
    pub fn with_write_lock<T>(&self, path: &RelPath, body: impl FnOnce() -> Result<T>) -> Result<T> {
        let file = self.open_lock_file(path)?;
        acquire(&file, path, LOCK_WAIT_LIMIT)?;

        let result = body();
        let _ = FileExt::unlock(&file);
        result
    }

    /// Where this vault's write lock lives.
    ///
    /// Named by the hash of the vault's canonical path, so every process that
    /// opens the same directory — however it spelled the path — agrees on the
    /// file, and two different vaults never share one.
    ///
    /// It sits in the per-user application-data directory rather than the cache
    /// directory. A cache is something the system may purge, and a purge that
    /// unlinks a held lock file is the same divergence that made sidecars
    /// unsafe to clean up.
    pub fn lock_path(&self) -> Utf8PathBuf {
        self.data_dir.join("locks").join(format!("{}.lock", self.key()))
    }

    /// Where this vault's note-lock rules live (see [`crate::notelocks`]).
    ///
    /// Beside the write lock for the same reason and under the same key: they
    /// are Heimdall's state about the vault, not part of it.
    pub fn state_path(&self) -> Utf8PathBuf {
        self.data_dir.join("vaults").join(format!("{}.json", self.key()))
    }

    /// The name every per-vault file is keyed by: a hash of the canonical root.
    fn key(&self) -> String {
        vault_key(&self.root)
    }

    /// Open (creating if needed) the file this vault's lock lives on.
    ///
    /// One plain `O_CREAT` open, which is all that is needed: on an absolute
    /// path this is a single atomic syscall, so every racing process either
    /// creates the file or opens the one that already exists, and all of them
    /// end up on the same inode.
    ///
    /// That was not true when the lock lived inside the vault. A directory
    /// capability resolves a path one component at a time, so `O_CREAT` alone
    /// was not atomic there and could report `NotFound` for a leaf another
    /// writer was creating at that instant — which surfaced to users as a bogus
    /// NOT_FOUND on a perfectly good write, and needed a retry loop to absorb.
    fn open_lock_file(&self, target: &RelPath) -> Result<std::fs::File> {
        let context = format!("lock {target}");
        let lock_path = self.lock_path();

        if let Some(parent) = lock_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|err| Error::from_io(&context, &err))?;
        }

        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)
            .map_err(|err| Error::from_io(&context, &err))
    }

    /// Flush a directory entry so a rename or create survives a crash.
    /// Best effort: some platforms and filesystems reject directory fsync.
    fn sync_dir(&self, path: &RelPath) {
        let opened = if path.is_root() {
            self.dir.try_clone()
        } else {
            self.dir.open_dir(path.as_str())
        };
        if let Ok(dir) = opened {
            let _ = dir.into_std_file().sync_all();
        }
    }
}

/// Removes a write temp file unless the write reached its rename.
struct TempGuard<'a> {
    vault: &'a Vault,
    path: Option<RelPath>,
}

impl TempGuard<'_> {
    fn disarm(mut self) {
        self.path = None;
    }
}

impl Drop for TempGuard<'_> {
    fn drop(&mut self) {
        if let Some(path) = &self.path {
            let _ = self.vault.dir.remove_file(path.as_str());
        }
    }
}

/// Take an exclusive lock, giving up rather than waiting forever.
///
/// Contention here is short by construction — a holder only compares a revision
/// and renames a file — so exceeding the deadline means another process is
/// stuck, and reporting that beats inheriting its hang.
fn acquire(file: &std::fs::File, target: &RelPath, wait_limit: Duration) -> Result<()> {
    let deadline = Instant::now() + wait_limit;
    loop {
        match file.try_lock_exclusive() {
            Ok(true) => return Ok(()),
            Ok(false) if Instant::now() < deadline => std::thread::sleep(LOCK_POLL_INTERVAL),
            Ok(false) => {
                return Err(Error::io_error(format!(
                    "another process has been writing to this vault for more than {} seconds; \
                     \"{target}\" was not written",
                    wait_limit.as_secs_f32()
                ))
                .with_detail("path", target.as_str()))
            }
            Err(err) => return Err(Error::from_io(&format!("lock {target}"), &err)),
        }
    }
}

/// The name a vault's files in the application-data directory are keyed by:
/// a hash of its canonical root.
///
/// A free function as well as [`Vault`]'s, so the files of a vault whose folder
/// is gone — which can no longer be opened — can still be found and removed.
pub fn vault_key(root: &Utf8Path) -> String {
    blake3::hash(root.as_str().as_bytes()).to_hex().to_string()
}

/// Whether a vault root has been deleted, as opposed to being out of reach.
///
/// Only a lookup that fails with "not found" counts. A macOS privacy refusal
/// is a folder that exists and this process may not enter; a root on an
/// external drive whose volume is not mounted is a drive that is unplugged.
/// Forgetting either would throw away a vault the user still has.
pub fn vault_is_gone(root: &Utf8Path) -> bool {
    match std::fs::symlink_metadata(root.as_std_path()) {
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => !on_unmounted_volume(root),
        _ => false,
    }
}

/// Whether `root` lies under `/Volumes/<name>` and that volume is not there.
fn on_unmounted_volume(root: &Utf8Path) -> bool {
    let mut components = root.components();
    let (Some(first), Some(second), Some(volume)) =
        (components.next(), components.next(), components.next())
    else {
        return false;
    };
    first.as_str() == "/"
        && second.as_str() == "Volumes"
        && !Utf8Path::new("/Volumes").join(volume.as_str()).exists()
}

/// Whether this build can make a note immutable at the filesystem level.
///
/// macOS has a per-file flag the owner sets without privileges and every
/// writer — editors, shells, `rename(2)`, `unlink(2)` — must respect.
/// Elsewhere the equivalents need administrator rights or do not stop a
/// rename, so a lock there binds Heimdall's own writers only, and `lock` says
/// so (`protection: unsupported`).
pub const FILE_PROTECTION: bool = cfg!(target_os = "macos");

#[cfg(target_os = "macos")]
mod osflags {
    use std::os::fd::AsRawFd;
    use std::os::macos::fs::MetadataExt;

    use cap_std::fs::Dir;

    pub(super) fn is_immutable(dir: &Dir, path: &str) -> bool {
        let Ok(file) = dir.open(path) else {
            return false;
        };
        let file = file.into_std();
        file.metadata()
            .is_ok_and(|meta| meta.is_file() && meta.st_flags() & libc::UF_IMMUTABLE != 0)
    }

    pub(super) fn set_immutable(dir: &Dir, path: &str, immutable: bool) -> std::io::Result<bool> {
        // Opened read-only: changing a file's flags needs ownership, not
        // write access, and a locked file cannot be opened for writing.
        let file = dir.open(path)?.into_std();
        let meta = file.metadata()?;
        if !meta.is_file() {
            return Ok(false);
        }
        let flags = meta.st_flags();
        let wanted = if immutable {
            flags | libc::UF_IMMUTABLE
        } else {
            flags & !libc::UF_IMMUTABLE
        };
        if wanted == flags {
            return Ok(false);
        }
        // SAFETY: `file` owns a valid descriptor for the duration of the call.
        if unsafe { libc::fchflags(file.as_raw_fd(), wanted) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(true)
    }
}

#[cfg(not(target_os = "macos"))]
mod osflags {
    use cap_std::fs::Dir;

    pub(super) fn is_immutable(_dir: &Dir, _path: &str) -> bool {
        false
    }

    pub(super) fn set_immutable(_dir: &Dir, _path: &str, _immutable: bool) -> std::io::Result<bool> {
        Ok(false)
    }
}

/// Resolve a vault path to its canonical form.
///
/// The lock key has to be one both processes compute identically, and a path as
/// typed is not that: `~/vault`, `./vault`, and a symlink to it are three
/// spellings of one directory.
pub(crate) fn canonicalize(root: &Utf8Path) -> Result<Utf8PathBuf> {
    let resolved = std::fs::canonicalize(root.as_std_path())
        .map_err(|err| Error::from_io("resolve vault path", &err))?;
    Utf8PathBuf::from_path_buf(resolved)
        .map_err(|_| Error::invalid_input("vault path is not valid UTF-8"))
}

/// `cap-std` addresses the directory itself as `.`, not as an empty path.
fn resolve(path: &RelPath) -> &str {
    if path.is_root() {
        "."
    } else {
        path.as_str()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::ErrorCode;

    fn temp_vault() -> (tempfile::TempDir, Vault) {
        let dir = tempfile::tempdir().unwrap();
        let root = Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).unwrap();
        let vault = Vault::open(&root).unwrap();
        (dir, vault)
    }

    fn rel(raw: &str) -> RelPath {
        RelPath::parse(raw).unwrap()
    }

    #[test]
    fn opening_a_missing_directory_is_not_found() {
        let err = Vault::open(Utf8Path::new("/nonexistent-heimdall-vault")).unwrap_err();
        assert_eq!(err.code, ErrorCode::NotFound);
    }

    #[test]
    fn atomic_write_replaces_content_and_reports_the_stored_revision() {
        let (_tmp, vault) = temp_vault();
        let path = rel("note.md");

        let first = vault.atomic_write(&path, b"one").unwrap();
        assert_eq!(vault.read(&path).unwrap(), b"one");
        let second = vault.atomic_write(&path, b"two").unwrap();
        assert_eq!(vault.read(&path).unwrap(), b"two");

        assert_ne!(first, second);
        assert_eq!(second, Revision::of_bytes(b"two"));
    }

    #[test]
    fn atomic_write_leaves_no_temp_files_behind() {
        let (_tmp, vault) = temp_vault();
        vault.atomic_write(&rel("note.md"), b"content").unwrap();
        let names: Vec<_> = vault
            .children(&RelPath::root())
            .unwrap()
            .into_iter()
            .map(|c| c.name)
            .collect();
        assert_eq!(names, vec!["note.md"]);
    }

    #[test]
    fn create_new_refuses_to_replace_an_existing_file() {
        let (_tmp, vault) = temp_vault();
        let path = rel("entry.md");
        vault.create_new(&path, b"first").unwrap();

        let err = vault.create_new(&path, b"second").unwrap_err();
        assert_eq!(err.code, ErrorCode::AlreadyExists);
        assert_eq!(vault.read(&path).unwrap(), b"first");
    }

    #[test]
    fn create_dir_all_reports_only_the_directories_it_made() {
        let (_tmp, vault) = temp_vault();
        let created = vault.create_dir_all(&rel("ideas/drafts/old")).unwrap();
        assert_eq!(
            created.iter().map(|p| p.as_str()).collect::<Vec<_>>(),
            ["ideas", "ideas/drafts", "ideas/drafts/old"]
        );
        // Re-running creates nothing.
        assert!(vault.create_dir_all(&rel("ideas/drafts")).unwrap().is_empty());
    }

    #[test]
    fn children_are_sorted_and_carry_resolved_metadata() {
        let (_tmp, vault) = temp_vault();
        vault.create_dir_all(&rel("projects")).unwrap();
        vault.atomic_write(&rel("b.md"), b"bb").unwrap();
        vault.atomic_write(&rel("a.md"), b"a").unwrap();

        let children = vault.children(&RelPath::root()).unwrap();
        let names: Vec<_> = children.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["a.md", "b.md", "projects"]);
        assert_eq!(children[0].meta.size_bytes, 1);
        assert_eq!(children[1].meta.size_bytes, 2);
        assert!(children[2].is_dir);
    }

    #[test]
    fn reading_a_directory_is_a_caller_error() {
        let (_tmp, vault) = temp_vault();
        vault.create_dir_all(&rel("projects")).unwrap();
        assert_eq!(
            vault.read(&rel("projects")).unwrap_err().code,
            ErrorCode::InvalidInput
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_pointing_out_of_the_vault_cannot_be_read_or_listed() {
        let (tmp, vault) = temp_vault();
        let outside = tmp.path().parent().unwrap().join("heimdall-outside-secret.md");
        std::fs::write(&outside, b"secret").unwrap();
        std::os::unix::fs::symlink(&outside, tmp.path().join("escape.md")).unwrap();

        let err = vault.read(&rel("escape.md")).unwrap_err();
        assert_eq!(err.code, ErrorCode::PathOutsideVault);

        let names: Vec<_> = vault
            .children(&RelPath::root())
            .unwrap()
            .into_iter()
            .map(|c| c.name)
            .collect();
        assert!(!names.contains(&"escape.md".to_string()), "{names:?}");

        std::fs::remove_file(outside).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn traversal_through_a_symlinked_directory_cannot_escape() {
        let (tmp, vault) = temp_vault();
        std::os::unix::fs::symlink(tmp.path().parent().unwrap(), tmp.path().join("up")).unwrap();
        assert_eq!(
            vault.read(&rel("up/anything.md")).unwrap_err().code,
            ErrorCode::PathOutsideVault
        );
    }

    #[test]
    fn a_file_lock_is_released_when_the_body_returns() {
        let (_tmp, vault) = temp_vault();
        let path = rel("note.md");
        vault.atomic_write(&path, b"one").unwrap();

        let value = vault.with_write_lock(&path, || Ok(7)).unwrap();
        assert_eq!(value, 7);
        // Re-acquiring immediately proves the previous lock was dropped.
        vault.with_write_lock(&path, || Ok(())).unwrap();
    }

    #[test]
    fn a_lock_failure_inside_the_body_still_releases_the_lock() {
        let (_tmp, vault) = temp_vault();
        let path = rel("note.md");
        vault.atomic_write(&path, b"one").unwrap();

        let err = vault
            .with_write_lock(&path, || Err::<(), _>(Error::internal("boom")))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::InternalError);
        vault.with_write_lock(&path, || Ok(())).unwrap();
    }

    #[test]
    fn waiting_for_a_stuck_holder_gives_up_instead_of_hanging() {
        let (_tmp, vault) = temp_vault();
        let path = rel("note.md");
        vault.atomic_write(&path, b"one").unwrap();

        // Hold the file this vault actually locks, the way a wedged process
        // would, then prove a waiter reports it rather than blocking forever.
        // Going through `lock_path` is the point: a test that held some other
        // file would pass while proving nothing.
        let lock = vault.lock_path();
        std::fs::create_dir_all(lock.parent().unwrap()).unwrap();
        let open = |create: bool| {
            std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(create)
                .truncate(false)
                .open(&lock)
                .unwrap()
        };
        let holder = open(true);
        holder.lock_exclusive().unwrap();

        // `acquire` directly rather than `with_write_lock`, with a deadline of
        // milliseconds: the production limit is ten seconds, and a test that
        // waits it out is ten seconds every run to learn nothing extra.
        let waiter = open(false);
        let err = acquire(&waiter, &path, Duration::from_millis(50)).unwrap_err();
        assert_eq!(err.code, ErrorCode::IoError);
        assert_eq!(err.details["path"], "note.md");

        // Once the holder lets go, the same waiter succeeds.
        FileExt::unlock(&holder).unwrap();
        acquire(&waiter, &path, Duration::from_millis(50)).unwrap();
    }

    #[test]
    #[cfg(unix)]
    fn one_vault_named_two_ways_shares_one_lock() {
        // The whole point of canonicalising the root. Two processes naming one
        // vault differently must derive the same lock file; if they did not,
        // each would take a lock of its own, exclude nothing, and the lost
        // write the lock exists to prevent would be back.
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("vault");
        std::fs::create_dir(&real).unwrap();
        let link = dir.path().join("by-another-name");
        std::os::unix::fs::symlink(&real, &link).unwrap();

        let direct = Vault::open(&Utf8PathBuf::from_path_buf(real).unwrap()).unwrap();
        let through_link = Vault::open(&Utf8PathBuf::from_path_buf(link).unwrap()).unwrap();

        assert_eq!(direct.lock_path(), through_link.lock_path());
    }

    #[test]
    fn two_vaults_never_share_a_lock() {
        let (_a, one) = temp_vault();
        let (_b, other) = temp_vault();
        assert_ne!(one.lock_path(), other.lock_path());
    }

    #[test]
    fn locking_writes_nothing_into_the_vault() {
        // The reason the lock moved out of the vault at all. A sidecar could
        // never be safely removed, so every note ever written left one behind.
        let (tmp, vault) = temp_vault();
        let path = rel("note.md");
        vault.atomic_write(&path, b"one").unwrap();
        vault.with_write_lock(&path, || Ok(())).unwrap();

        let mut stack = vec![tmp.path().to_path_buf()];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap() {
                let entry = entry.unwrap();
                let name = entry.file_name().to_string_lossy().into_owned();
                assert!(!name.ends_with(".lock"), "left {} in the vault", entry.path().display());
                if entry.file_type().unwrap().is_dir() {
                    stack.push(entry.path());
                }
            }
        }
        assert!(vault.lock_path().exists(), "the lock itself must exist, outside the vault");
    }

    #[test]
    fn legacy_lock_sidecars_are_never_listed_as_content() {
        // Heimdall no longer writes these, but a vault used before the lock
        // moved out still has one beside every note it ever wrote, and those
        // must stay invisible rather than appearing as content.
        let (_tmp, vault) = temp_vault();
        let path = rel("note.md");
        vault.atomic_write(&path, b"one").unwrap();
        vault.atomic_write(&rel(".note.md.lock"), b"").unwrap();

        let listable: Vec<_> = vault
            .children(&RelPath::root())
            .unwrap()
            .into_iter()
            .map(|c| c.name)
            .filter(|name| crate::paths::is_listable(name))
            .collect();
        assert_eq!(listable, vec!["note.md"]);
    }
}

#[cfg(test)]
mod thread_safety {
    /// The MCP server holds one vault across concurrent tool calls and runs
    /// each blocking operation on a worker thread, so this is a load-bearing
    /// property rather than an incidental one.
    #[test]
    fn a_vault_can_be_shared_across_threads() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<super::Vault>();
    }
}

//! The vault capability and every filesystem operation Heimdall performs.
//!
//! The vault is opened once as a `cap-std` directory capability; all
//! descendants resolve beneath it. Containment is enforced by the capability
//! itself, not by comparing path strings (SPEC §14), so `..` components,
//! absolute children, and symlinks pointing out of the vault fail at the
//! syscall boundary rather than being filtered by hand.

use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use camino::{Utf8Path, Utf8PathBuf};
use cap_std::ambient_authority;
use cap_std::fs::{Dir, OpenOptions};
use fs4::fs_std::FileExt;
use time::OffsetDateTime;

use crate::errors::{Error, Result};
use crate::paths::{self, RelPath};
use crate::revisions::Revision;
use crate::timestamps;

/// Distinguishes concurrent temp files written by one process.
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// How many times to retry opening a lock sidecar that competing writers are
/// creating at the same moment. Each loss is another writer's win, so the
/// contention this absorbs is bounded by the number of concurrent writers.
const LOCK_OPEN_ATTEMPTS: usize = 64;

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
    root: Utf8PathBuf,
}

impl Vault {
    /// Open an existing directory as a vault capability.
    ///
    /// This does not check initialization; call [`Vault::ensure_initialized`]
    /// before any domain operation that reads or writes managed content.
    pub fn open(root: &Utf8Path) -> Result<Self> {
        let dir = Dir::open_ambient_dir(root.as_std_path(), ambient_authority()).map_err(|err| {
            match err.kind() {
                std::io::ErrorKind::NotFound => {
                    Error::not_found("vault directory does not exist")
                }
                std::io::ErrorKind::NotADirectory => {
                    Error::invalid_input("vault path is not a directory")
                }
                _ => Error::from_io("open vault", &err),
            }
        })?;
        Ok(Self {
            dir,
            root: root.to_owned(),
        })
    }

    /// The path this vault was opened from, as supplied by the caller.
    pub fn root(&self) -> &Utf8Path {
        &self.root
    }

    /// Verify the managed `aios/` structure exists, or explain how to fix it.
    ///
    /// Both `agents.md` and `AGENTS.md` are accepted (SPEC §6); a vault created
    /// on a case-insensitive filesystem keeps working when copied to Linux.
    pub fn ensure_initialized(&self) -> Result<()> {
        let mut missing: Vec<String> = Vec::new();

        for dir in paths::REQUIRED_DIRS {
            if !self.dir.is_dir(dir) {
                missing.push(format!("{dir}/"));
            }
        }
        if self.agents_file().is_none() {
            missing.push(paths::AGENTS_FILE_CANDIDATES[0].to_string());
        }
        if !self.dir.is_file(paths::MAIN_MEMORY_FILE) {
            missing.push(paths::MAIN_MEMORY_FILE.to_string());
        }

        if missing.is_empty() {
            return Ok(());
        }
        Err(Error::not_initialized(
            "this vault has no complete aios/ structure; run \"heimdall create\" against it \
             (or initialize it from the desktop app) to add the missing managed content",
        )
        .with_detail("missing", missing))
    }

    /// The agent instructions path actually present in this vault, if any.
    pub fn agents_file(&self) -> Option<RelPath> {
        paths::AGENTS_FILE_CANDIDATES
            .iter()
            .find(|candidate| self.dir.is_file(candidate))
            .and_then(|candidate| RelPath::parse(candidate).ok())
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
    /// discipline `write_memory` uses around revision comparison. Callers are
    /// responsible for holding that lock.
    ///
    /// Being honest about what this does and does not guarantee: it serializes
    /// Heimdall processes against each other, not against Obsidian writing into
    /// the same vault. That boundary is already accepted — SPEC §8 says
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

    /// Remove one file.
    ///
    /// Heimdall never calls this to destroy user content — `delete_path` moves
    /// notes into `.trash/` instead — but the primitive is needed to clear the
    /// lock sidecar left beside a file that has been moved away.
    pub fn remove_file(&self, path: &RelPath) -> Result<()> {
        self.dir
            .remove_file(path.as_str())
            .map_err(|err| Error::from_io_path(&format!("remove {path}"), &err))?;
        self.sync_dir(&path.parent());
        Ok(())
    }

    /// Drop the lock sidecar beside a file that has just moved away.
    ///
    /// A sidecar normally outlives the operation that made it, which is correct:
    /// removing one while another process might be about to open it is a race,
    /// and `paths::is_heimdall_internal` keeps them out of every listing anyway.
    /// A sidecar whose file has been renamed away is different — nothing will
    /// ever open it again, and leaving debris in a user's vault is not
    /// acceptable. Best-effort by design: failing to tidy up is not a reason to
    /// fail a move the filesystem already completed.
    pub fn discard_lock_sidecar(&self, path: &RelPath) {
        let Some(name) = path.file_name() else {
            return;
        };
        let sidecar = path.parent().join(&format!(".{name}.lock"));
        if self.is_file(&sidecar) {
            let _ = self.remove_file(&sidecar);
        }
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

    /// Run `body` holding a cross-process exclusive lock for `path`.
    ///
    /// The lock lives on a sidecar `.<name>.lock` file rather than the target
    /// itself: `atomic_write` renames a new inode over the target, which would
    /// otherwise leave each writer holding a lock on a different file.
    ///
    /// Waiting for the lock is bounded (SPEC §12). A blocking acquire would
    /// hang for as long as some other process — possibly one that is wedged, or
    /// stopped under a debugger — chooses to hold it, and a server cannot offer
    /// a bounded operation duration on top of an unbounded wait.
    pub fn with_file_lock<T>(&self, path: &RelPath, body: impl FnOnce() -> Result<T>) -> Result<T> {
        let name = path
            .file_name()
            .ok_or_else(|| Error::internal("lock target has no filename"))?;
        let lock_path = path.parent().join(&format!(".{name}.lock"));

        let file = self.open_lock_file(&lock_path, path)?;
        acquire(&file, path, LOCK_WAIT_LIMIT)?;

        let result = body();
        let _ = FileExt::unlock(&file);
        result
    }

    /// Open (creating if needed) the sidecar a lock lives on.
    ///
    /// Opening with `O_CREAT` but no `O_EXCL` is not safe to race here:
    /// resolving a path one component at a time, as a directory capability
    /// must, can report `NotFound` for a leaf another process is creating at
    /// that instant. Splitting the two cases keeps every step a single atomic
    /// open — take the file if it exists, else claim it with `O_EXCL` — and
    /// retries when a competing creator makes both steps miss.
    fn open_lock_file(&self, lock_path: &RelPath, target: &RelPath) -> Result<std::fs::File> {
        let context = format!("lock {target}");

        let mut existing = OpenOptions::new();
        existing.read(true).write(true);
        let mut claim = OpenOptions::new();
        claim.read(true).write(true).create_new(true);

        for _ in 0..LOCK_OPEN_ATTEMPTS {
            match self.dir.open_with(lock_path.as_str(), &existing) {
                Ok(file) => return Ok(file.into_std()),
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(err) => return Err(Error::from_io_path(&context, &err)),
            }

            match self.dir.open_with(lock_path.as_str(), &claim) {
                Ok(file) => return Ok(file.into_std()),
                // Another process created it first, or is creating it right
                // now; either way it exists on the next pass.
                Err(err)
                    if matches!(
                        err.kind(),
                        std::io::ErrorKind::AlreadyExists | std::io::ErrorKind::NotFound
                    ) => {}
                Err(err) => return Err(Error::from_io_path(&context, &err)),
            }
            std::thread::yield_now();
        }

        Err(Error::io_error(format!(
            "could not acquire a lock for \"{target}\" after {LOCK_OPEN_ATTEMPTS} attempts"
        ))
        .with_detail("path", target.as_str()))
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
                    "another process has held the lock on \"{target}\" for more than {} seconds",
                    wait_limit.as_secs_f32()
                ))
                .with_detail("path", target.as_str()))
            }
            Err(err) => return Err(Error::from_io(&format!("lock {target}"), &err)),
        }
    }
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
    fn an_empty_directory_is_not_an_initialized_vault() {
        let (_tmp, vault) = temp_vault();
        let err = vault.ensure_initialized().unwrap_err();
        assert_eq!(err.code, ErrorCode::NotInitialized);
        assert!(err.message.contains("heimdall create"), "{}", err.message);
        assert!(err.details["missing"].as_array().unwrap().len() > 1);
    }

    #[test]
    fn either_agents_filename_satisfies_initialization() {
        for agents in ["aios/agents.md", "aios/AGENTS.md"] {
            let (_tmp, vault) = temp_vault();
            for dir in paths::REQUIRED_DIRS {
                vault.create_dir_all(&rel(dir)).unwrap();
            }
            vault.atomic_write(&rel(agents), b"See [[memory]]").unwrap();
            vault
                .atomic_write(&rel(paths::MAIN_MEMORY_FILE), b"# Memory\n")
                .unwrap();

            vault.ensure_initialized().unwrap();
            assert!(vault.agents_file().is_some());
        }
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
        let created = vault.create_dir_all(&rel("aios/memories/extended")).unwrap();
        assert_eq!(
            created.iter().map(|p| p.as_str()).collect::<Vec<_>>(),
            ["aios", "aios/memories", "aios/memories/extended"]
        );
        // Re-running creates nothing.
        assert!(vault.create_dir_all(&rel("aios/memories")).unwrap().is_empty());
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
        let path = rel("memory.md");
        vault.atomic_write(&path, b"one").unwrap();

        let value = vault.with_file_lock(&path, || Ok(7)).unwrap();
        assert_eq!(value, 7);
        // Re-acquiring immediately proves the previous lock was dropped.
        vault.with_file_lock(&path, || Ok(())).unwrap();
    }

    #[test]
    fn a_lock_failure_inside_the_body_still_releases_the_lock() {
        let (_tmp, vault) = temp_vault();
        let path = rel("memory.md");
        vault.atomic_write(&path, b"one").unwrap();

        let err = vault
            .with_file_lock(&path, || Err::<(), _>(Error::internal("boom")))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::InternalError);
        vault.with_file_lock(&path, || Ok(())).unwrap();
    }

    #[test]
    fn waiting_for_a_stuck_holder_gives_up_instead_of_hanging() {
        let (tmp, vault) = temp_vault();
        let path = rel("memory.md");
        vault.atomic_write(&path, b"one").unwrap();

        // Hold the sidecar the way a wedged process would, then prove a second
        // waiter reports the situation rather than blocking forever.
        let sidecar = tmp.path().join(".memory.md.lock");
        let holder = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&sidecar)
            .unwrap();
        holder.lock_exclusive().unwrap();

        let waiter = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&sidecar)
            .unwrap();
        let err = acquire(&waiter, &path, Duration::from_millis(50)).unwrap_err();

        assert_eq!(err.code, ErrorCode::IoError);
        assert_eq!(err.details["path"], "memory.md");

        // Once the holder lets go, the same waiter succeeds.
        FileExt::unlock(&holder).unwrap();
        acquire(&waiter, &path, Duration::from_millis(50)).unwrap();
    }

    #[test]
    fn lock_sidecars_are_never_listed_as_content() {
        let (_tmp, vault) = temp_vault();
        let path = rel("memory.md");
        vault.atomic_write(&path, b"one").unwrap();
        vault.with_file_lock(&path, || Ok(())).unwrap();

        let listable: Vec<_> = vault
            .children(&RelPath::root())
            .unwrap()
            .into_iter()
            .map(|c| c.name)
            .filter(|name| paths::is_listable(name))
            .collect();
        assert_eq!(listable, vec!["memory.md"]);
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

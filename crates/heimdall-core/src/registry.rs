//! Which folders are vaults, so the shell can work from inside one (SPEC §7).
//!
//! A vault is a plain folder of Markdown with nothing of Heimdall's in it, so
//! nothing on disk can say "this is a vault root". The registry says it
//! instead, from the application-data directory: `heimdall create` records a
//! vault, and so does any command that names one with `--vault`, which is how
//! vaults that existed before the registry join it without a migration.
//!
//! `heimdall read` run from `~/Notes/projects` walks up to the nearest
//! registered root and reads `projects` — the current folder — without being
//! told where the vault is. Paths typed there are relative to that folder, the
//! way every other shell command treats them.

use std::time::{Duration, Instant};

use camino::{Utf8Component, Utf8Path, Utf8PathBuf};
use fs4::fs_std::FileExt;
use serde::{Deserialize, Serialize};

use crate::appdata;
use crate::errors::{Error, Result};
use crate::paths::RelPath;
use crate::storage::{self, Vault};

/// How long to wait for another process that is registering a vault.
const REGISTRY_WAIT_LIMIT: Duration = Duration::from_secs(10);

#[derive(Debug, Default, Serialize, Deserialize)]
struct Stored {
    #[serde(default)]
    vaults: Vec<Utf8PathBuf>,
}

/// A vault found from a working directory, and where in it that directory is.
#[derive(Debug)]
pub struct Located {
    pub vault: Vault,
    /// The working directory, relative to the vault root. The root itself when
    /// the shell is standing at the top of the vault.
    pub cwd: RelPath,
}

/// Every registered vault root, sorted.
pub fn list(data_dir: &Utf8Path) -> Result<Vec<Utf8PathBuf>> {
    Ok(load(&registry_path(data_dir))?.vaults)
}

/// Record `vault` so the shell can find it from inside. Idempotent.
pub fn register(vault: &Vault) -> Result<()> {
    let path = registry_path(vault.data_dir());
    let root = vault.root().to_owned();

    if load(&path)?.vaults.contains(&root) {
        return Ok(());
    }

    // Two processes registering at once must not each write a list missing
    // the other's vault, so the read-modify-write happens under a lock.
    with_registry_lock(vault.data_dir(), || {
        let mut stored = load(&path)?;
        if !stored.vaults.contains(&root) {
            stored.vaults.push(root.clone());
            stored.vaults.sort();
            appdata::write_json(&path, &stored)?;
        }
        Ok(())
    })
}

/// Find the vault containing `cwd`: the longest registered root that is `cwd`
/// or one of its parents, so a vault nested inside another is found as itself.
pub fn locate(cwd: &Utf8Path, data_dir: &Utf8Path) -> Result<Located> {
    let cwd = storage::canonicalize(cwd)?;
    let stored = load(&registry_path(data_dir))?;

    let root = stored
        .vaults
        .iter()
        .filter(|root| cwd.starts_with(root))
        .max_by_key(|root| root.components().count())
        .ok_or_else(|| {
            Error::not_initialized(
                "the working directory is not inside a Heimdall vault; pass --vault <path>, \
                 or run \"heimdall create\" to make this folder one",
            )
        })?;

    let vault = Vault::open_with_data_dir(root, data_dir)?;
    let relative = cwd
        .strip_prefix(vault.root())
        .map_err(|_| Error::internal("the working directory left its vault"))?;
    let cwd = RelPath::parse(&to_slashes(relative))?;
    Ok(Located { vault, cwd })
}

/// Turn a path typed at the shell into a vault path.
///
/// - An absolute path must lie inside the vault, and is re-expressed relative
///   to it — so pasting a note's full path works.
/// - A relative path is taken from `base`: the working directory when the vault
///   was found by walking up from it, the vault root when `--vault` named it.
///   `..` may climb out of the working directory but never out of the vault.
///
/// Nothing here grants access: the result is an ordinary [`RelPath`], and the
/// vault's directory capability still decides what it can reach.
pub fn resolve_shell_path(vault_root: &Utf8Path, base: &RelPath, input: &str) -> Result<RelPath> {
    let typed = Utf8Path::new(input);
    let (mut components, rest): (Vec<String>, Utf8PathBuf) = if typed.is_absolute() {
        let absolute = canonicalize_existing_prefix(typed)?;
        let inside = absolute.strip_prefix(vault_root).map_err(|_| {
            Error::path_outside_vault("that path is not inside this vault")
                .with_detail("parameter", "path")
        })?;
        (Vec::new(), inside.to_owned())
    } else {
        (
            base.components().map(str::to_string).collect(),
            typed.to_owned(),
        )
    };

    for component in rest.components() {
        match component {
            Utf8Component::CurDir => {}
            Utf8Component::ParentDir => {
                if components.pop().is_none() {
                    return Err(Error::path_outside_vault("that path climbs out of the vault")
                        .with_detail("parameter", "path"));
                }
            }
            Utf8Component::Normal(name) => components.push(name.to_string()),
            Utf8Component::RootDir | Utf8Component::Prefix(_) => {}
        }
    }
    RelPath::parse(&components.join("/"))
}

/// Canonicalize as much of an absolute path as exists, and append the rest.
///
/// A note about to be written does not exist yet, so the whole path cannot be
/// canonicalized — but its folder must be, or `/tmp/vault` and
/// `/private/tmp/vault` (one directory on macOS) would disagree about whether
/// the path is inside the vault.
fn canonicalize_existing_prefix(path: &Utf8Path) -> Result<Utf8PathBuf> {
    let mut existing = path.to_owned();
    let mut tail: Vec<String> = Vec::new();
    while !existing.exists() {
        match (existing.file_name(), existing.parent()) {
            (Some(name), Some(parent)) => {
                tail.push(name.to_string());
                existing = parent.to_owned();
            }
            _ => break,
        }
    }
    let mut resolved = storage::canonicalize(&existing)?;
    for name in tail.iter().rev() {
        resolved.push(name);
    }
    Ok(resolved)
}

fn to_slashes(path: &Utf8Path) -> String {
    path.components()
        .map(|c| c.as_str().to_string())
        .collect::<Vec<_>>()
        .join("/")
}

fn registry_path(data_dir: &Utf8Path) -> Utf8PathBuf {
    data_dir.join("vaults.json")
}

fn load(path: &Utf8Path) -> Result<Stored> {
    Ok(appdata::read_json(path)?.unwrap_or_default())
}

fn with_registry_lock<T>(data_dir: &Utf8Path, body: impl FnOnce() -> Result<T>) -> Result<T> {
    with_state_lock(data_dir, "registry.lock", "the vault registry", body)
}

/// Run `body` holding a short, named lock in the application-data directory,
/// for a read-modify-write of one of Heimdall's own state files.
pub(crate) fn with_state_lock<T>(
    data_dir: &Utf8Path,
    file_name: &str,
    what: &str,
    body: impl FnOnce() -> Result<T>,
) -> Result<T> {
    let lock_path = data_dir.join("locks").join(file_name);
    let context = format!("lock {what}");
    if let Some(parent) = lock_path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| Error::from_io(&context, &err))?;
    }
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .map_err(|err| Error::from_io(&context, &err))?;

    let deadline = Instant::now() + REGISTRY_WAIT_LIMIT;
    loop {
        match file.try_lock_exclusive() {
            Ok(true) => break,
            Ok(false) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(2)),
            Ok(false) => {
                return Err(Error::io_error(format!(
                    "another process has held {what} for more than 10 seconds"
                )))
            }
            Err(err) => return Err(Error::from_io(&context, &err)),
        }
    }
    let result = body();
    let _ = FileExt::unlock(&file);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::ErrorCode;

    fn rel(raw: &str) -> RelPath {
        RelPath::parse(raw).unwrap()
    }

    #[test]
    fn a_relative_path_is_taken_from_the_base_folder() {
        let root = Utf8Path::new("/v");
        assert_eq!(resolve_shell_path(root, &rel("projects"), "a.md").unwrap(), rel("projects/a.md"));
        assert_eq!(resolve_shell_path(root, &rel("projects"), "../ideas/b.md").unwrap(), rel("ideas/b.md"));
        assert_eq!(resolve_shell_path(root, &rel("projects"), ".").unwrap(), rel("projects"));
        assert_eq!(resolve_shell_path(root, &rel("projects"), "..").unwrap(), RelPath::root());
    }

    #[test]
    fn climbing_out_of_the_vault_is_refused() {
        let root = Utf8Path::new("/v");
        for input in ["..", "../../x.md", "a/../../x.md"] {
            let err = resolve_shell_path(root, &RelPath::root(), input).unwrap_err();
            assert_eq!(err.code, ErrorCode::PathOutsideVault, "{input}");
        }
    }

    #[test]
    fn an_absolute_path_inside_the_vault_becomes_vault_relative() {
        let dir = tempfile::tempdir().unwrap();
        let root = storage::canonicalize(Utf8Path::from_path(dir.path()).unwrap()).unwrap();
        std::fs::create_dir(root.join("projects")).unwrap();

        let typed = format!("{}/projects/new.md", dir.path().display());
        assert_eq!(
            resolve_shell_path(&root, &rel("ideas"), &typed).unwrap(),
            rel("projects/new.md")
        );

        let err = resolve_shell_path(&root, &RelPath::root(), "/etc/hosts").unwrap_err();
        assert_eq!(err.code, ErrorCode::PathOutsideVault);
    }
}

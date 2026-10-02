//! The vaults a person has shared with AI clients (SPEC §7).
//!
//! One MCP server serves every vault on this list, and only those. A tool call
//! picks one by its **name** — never by a path — so an agent can tell a user's
//! vaults apart without being handed a way to reach anything else on disk.
//! Which vaults are here, and what each is called, is the person's decision:
//! the list changes only through the shell or the desktop, never a tool.
//!
//! It is its own file (`agents.json`) rather than a field in the registry so
//! that an older Heimdall rewriting `vaults.json` cannot drop it.
//!
//! The server reads this file on every call, so sharing or unsharing a vault
//! takes effect at once, without the client restarting the server.

use camino::{Utf8Path, Utf8PathBuf};
use serde::{Deserialize, Serialize};

use crate::appdata;
use crate::errors::{Error, Result};
use crate::paths;
use crate::registry::{self, with_state_lock};
use crate::storage::Vault;

#[derive(Debug, Default, Serialize, Deserialize)]
struct Stored {
    #[serde(default)]
    vaults: Vec<SharedVault>,
}

/// One vault shared with AI clients.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SharedVault {
    /// What a tool call names it by. Unique, ignoring case.
    pub name: String,
    /// Its canonical root. Never shown to an agent.
    pub path: Utf8PathBuf,
}

fn file(data_dir: &Utf8Path) -> Utf8PathBuf {
    data_dir.join("agents.json")
}

fn load(data_dir: &Utf8Path) -> Result<Stored> {
    Ok(appdata::read_json(&file(data_dir))?.unwrap_or_default())
}

fn same_name(a: &str, b: &str) -> bool {
    a.to_lowercase() == b.to_lowercase()
}

/// Every shared vault, in name order.
pub fn list(data_dir: &Utf8Path) -> Result<Vec<SharedVault>> {
    let mut vaults = load(data_dir)?.vaults;
    vaults.sort_by_key(|vault| vault.name.to_lowercase());
    Ok(vaults)
}

/// The name `vault` is shared under, if it is shared.
pub fn name_of(vault: &Vault) -> Result<Option<String>> {
    Ok(load(vault.data_dir())?
        .vaults
        .into_iter()
        .find(|shared| shared.path == vault.root())
        .map(|shared| shared.name))
}

/// What `vault` is called in results: its shared name, or its folder name
/// when it is not shared. Checks that `requested`, when given, names it.
///
/// The adapter has already chosen the vault; this is the second look that
/// makes "resolved one vault, operated on another" impossible to ship.
pub fn confirm_name(vault: &Vault, requested: Option<&str>) -> Result<String> {
    let name = name_of(vault)?.unwrap_or_else(|| vault.name().to_string());
    match requested {
        Some(requested) if !same_name(requested, &name) => Err(Error::not_found(format!(
            "no shared vault is called \"{requested}\""
        ))
        .with_detail("parameter", "vault")),
        _ => Ok(name),
    }
}

/// Share `vault` with AI clients as `name` — its folder name when none is
/// given — or rename it if it is already shared. Also registers it.
///
/// A name another shared vault already uses, ignoring case, is refused when
/// it was asked for. When it was not, a numeric suffix keeps the folder name
/// recognisable: two folders called `notes` are shared as `notes` and
/// `notes 2`.
pub fn share(vault: &Vault, name: Option<&str>) -> Result<SharedVault> {
    let requested = match name {
        Some(raw) => Some(paths::validate_vault_name(raw)?.to_string()),
        None => None,
    };
    let root = vault.root().to_owned();
    let data_dir = vault.data_dir();

    let shared = with_state_lock(data_dir, "agents.lock", "the shared vaults", || {
        let mut stored = load(data_dir)?;
        let taken = |candidate: &str, stored: &Stored| {
            stored
                .vaults
                .iter()
                .any(|other| other.path != root && same_name(&other.name, candidate))
        };

        let name = match &requested {
            Some(name) if taken(name, &stored) => {
                return Err(Error::already_exists(format!(
                    "another shared vault is already called \"{name}\""
                ))
                .with_detail("parameter", "name"));
            }
            Some(name) => name.clone(),
            None => match stored.vaults.iter().find(|shared| shared.path == root) {
                Some(existing) => existing.name.clone(),
                None => {
                    let base = vault.name().to_string();
                    let mut candidate = base.clone();
                    let mut n = 2;
                    while taken(&candidate, &stored) {
                        candidate = format!("{base} {n}");
                        n += 1;
                    }
                    candidate
                }
            },
        };

        let entry = SharedVault {
            name,
            path: root.clone(),
        };
        stored.vaults.retain(|shared| shared.path != root);
        stored.vaults.push(entry.clone());
        stored.vaults.sort_by_key(|vault| vault.name.to_lowercase());
        appdata::write_json(&file(data_dir), &stored)?;
        Ok(entry)
    })?;

    registry::register(vault)?;
    Ok(shared)
}

/// Stop sharing `vault`. Returns whether it had been shared.
pub fn unshare(vault: &Vault) -> Result<bool> {
    let root = vault.root().to_owned();
    let data_dir = vault.data_dir();
    with_state_lock(data_dir, "agents.lock", "the shared vaults", || {
        let mut stored = load(data_dir)?;
        let before = stored.vaults.len();
        stored.vaults.retain(|shared| shared.path != root);
        if stored.vaults.len() == before {
            return Ok(false);
        }
        appdata::write_json(&file(data_dir), &stored)?;
        Ok(true)
    })
}

/// Pick the shared vault a tool call means.
///
/// - A name selects the shared vault of that name, ignoring case. A name that
///   is not on the list is `NOT_FOUND` whether or not some folder by that name
///   exists: only shared vaults are anyone's business here.
/// - No name is fine when exactly one vault is shared.
/// - With several shared, leaving it out is `INVALID_INPUT`, listing the
///   names — guessing is how a note ends up in the wrong vault.
pub fn select(data_dir: &Utf8Path, name: Option<&str>) -> Result<SharedVault> {
    let vaults = list(data_dir)?;
    let names: Vec<String> = vaults.iter().map(|vault| vault.name.clone()).collect();

    if vaults.is_empty() {
        return Err(Error::not_found(
            "no vaults are shared with AI clients yet; the user can share one in Heimdall",
        ));
    }
    match name {
        Some(name) => vaults
            .into_iter()
            .find(|vault| same_name(&vault.name, name))
            .ok_or_else(|| {
                Error::not_found(format!(
                    "no shared vault is called \"{name}\"; the shared vaults are: {}",
                    names.join(", ")
                ))
                .with_detail("parameter", "vault")
                .with_detail("vaults", names.clone())
            }),
        None if vaults.len() == 1 => Ok(vaults.into_iter().next().expect("one vault")),
        None => Err(Error::invalid_input(format!(
            "more than one vault is shared; pass \"vault\" with one of: {}",
            names.join(", ")
        ))
        .with_detail("parameter", "vault")
        .with_detail("vaults", names)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::ErrorCode;

    struct Fixture {
        _dirs: Vec<tempfile::TempDir>,
        data: Utf8PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let data = tempfile::tempdir().unwrap();
            let path = Utf8PathBuf::from_path_buf(data.path().to_path_buf()).unwrap();
            Self {
                _dirs: vec![data],
                data: path,
            }
        }

        /// A vault in a fresh folder called `folder`.
        fn vault(&mut self, folder: &str) -> Vault {
            let parent = tempfile::tempdir().unwrap();
            let root = parent.path().join(folder);
            std::fs::create_dir(&root).unwrap();
            let root = Utf8PathBuf::from_path_buf(root).unwrap();
            self._dirs.push(parent);
            Vault::open_with_data_dir(&root, &self.data).unwrap()
        }
    }

    #[test]
    fn a_vault_is_shared_under_its_folder_name_and_registered() {
        let mut fx = Fixture::new();
        let work = fx.vault("Work");
        let shared = share(&work, None).unwrap();
        assert_eq!(shared.name, "Work");
        assert_eq!(name_of(&work).unwrap().as_deref(), Some("Work"));
        let registry = std::fs::read_to_string(fx.data.join("vaults.json")).unwrap();
        assert!(registry.contains(work.root().as_str()));
    }

    #[test]
    fn two_folders_with_one_name_get_two_names() {
        let mut fx = Fixture::new();
        let a = fx.vault("notes");
        let b = fx.vault("Notes");
        assert_eq!(share(&a, None).unwrap().name, "notes");
        assert_eq!(share(&b, None).unwrap().name, "Notes 2");
    }

    #[test]
    fn a_name_in_use_is_refused_when_asked_for_and_names_are_validated() {
        let mut fx = Fixture::new();
        let a = fx.vault("a");
        let b = fx.vault("b");
        share(&a, Some("Work")).unwrap();
        assert_eq!(share(&b, Some("work")).unwrap_err().code, ErrorCode::AlreadyExists);
        assert_eq!(share(&b, Some("x/y")).unwrap_err().code, ErrorCode::InvalidInput);
        assert_eq!(share(&b, Some("")).unwrap_err().code, ErrorCode::InvalidInput);
        // Renaming a shared vault to its own name, in any case, is fine.
        assert_eq!(share(&a, Some("WORK")).unwrap().name, "WORK");
        assert_eq!(list(&fx.data).unwrap().len(), 1);
    }

    #[test]
    fn selection_needs_a_name_only_when_there_is_a_choice() {
        let mut fx = Fixture::new();
        assert_eq!(select(&fx.data, None).unwrap_err().code, ErrorCode::NotFound);

        let work = fx.vault("Work");
        share(&work, None).unwrap();
        assert_eq!(select(&fx.data, None).unwrap().name, "Work");
        assert_eq!(select(&fx.data, Some("work")).unwrap().name, "Work");

        let home = fx.vault("Home");
        share(&home, None).unwrap();
        let err = select(&fx.data, None).unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidInput);
        assert_eq!(err.details["vaults"], serde_json::json!(["Home", "Work"]));
    }

    #[test]
    fn an_unshared_vault_and_a_made_up_name_are_the_same_answer() {
        let mut fx = Fixture::new();
        let work = fx.vault("Work");
        let private = fx.vault("Private");
        share(&work, None).unwrap();
        share(&private, None).unwrap();
        unshare(&private).unwrap();

        let unshared = select(&fx.data, Some("Private")).unwrap_err();
        let invented = select(&fx.data, Some("Nowhere")).unwrap_err();
        assert_eq!(unshared.code, ErrorCode::NotFound);
        assert_eq!(unshared.details, {
            let mut d = invented.details.clone();
            d.insert("vaults".into(), serde_json::json!(["Work"]));
            d
        });
        // A path is only a name nobody shared, even the path of one that is.
        let path = select(&fx.data, Some(work.root().as_str())).unwrap_err();
        assert_eq!(path.code, ErrorCode::NotFound);
    }

    #[test]
    fn unsharing_twice_is_not_a_change_and_leaves_the_registry_alone() {
        let mut fx = Fixture::new();
        let work = fx.vault("Work");
        share(&work, None).unwrap();
        let registry = std::fs::read(fx.data.join("vaults.json")).unwrap();
        assert!(unshare(&work).unwrap());
        assert!(!unshare(&work).unwrap());
        assert_eq!(std::fs::read(fx.data.join("vaults.json")).unwrap(), registry);
        assert_eq!(name_of(&work).unwrap(), None);
    }
}

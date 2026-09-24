//! `heimdall create` — scaffold a new vault, or adopt an existing folder of
//! notes as one.
//!
//! Setup performed by a human, directly or through the desktop app. It is a
//! shell command only and is never exposed as an MCP tool (SPEC §7).
//!
//! Either way the vault is registered (see [`crate::registry`]), which is what
//! lets `heimdall read` find it from any folder inside.

use camino::Utf8PathBuf;
use serde::{Deserialize, Serialize};

use crate::errors::{Error, Result};
use crate::paths::{self, RelPath};
use crate::storage::Vault;
use crate::registry;
use crate::template;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateVaultRequest {
    /// One folder name, not a path.
    pub name: String,
    /// Where to create it. The CLI defaults this to the working directory.
    pub root: Utf8PathBuf,
}

/// Whether a new vault was built or an existing folder was adopted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CreateMode {
    /// The target was missing or empty: the full template was written.
    Scaffolded,
    /// The target already had content: nothing was written into it, and it was
    /// registered as a vault as it stands.
    Registered,
}

#[derive(Debug, Clone, Serialize)]
pub struct CreateVaultResponse {
    pub path: Utf8PathBuf,
    pub mode: CreateMode,
    /// Vault-relative paths written by this call, directories suffixed with `/`.
    pub created: Vec<String>,
}

pub fn create_vault(request: CreateVaultRequest) -> Result<CreateVaultResponse> {
    let name = paths::validate_vault_name(&request.name)?;
    let name_path = RelPath::parse(name)?;

    let root = Vault::open(&request.root).map_err(|err| match err.code {
        crate::errors::ErrorCode::NotFound => {
            Error::not_found("the --root directory does not exist")
        }
        _ => err,
    })?;

    if root.exists(&name_path) && !root.is_dir(&name_path) {
        return Err(Error::already_exists(format!(
            "\"{name}\" already exists in the root directory and is not a directory"
        ))
        .with_detail("name", name));
    }

    let target_created = if root.is_dir(&name_path) {
        false
    } else {
        root.create_dir_all(&name_path)?;
        true
    };

    let path = request.root.join(name);
    let outcome = build(&path);

    match outcome {
        Ok((mode, created)) => Ok(CreateVaultResponse { path, mode, created }),
        Err(err) => {
            // Roll back only what was empty: a directory this call created and
            // never populated. Anything with content is the user's now.
            if target_created {
                root.remove_dir_if_empty(&name_path);
            }
            Err(err)
        }
    }
}

fn build(path: &Utf8PathBuf) -> Result<(CreateMode, Vec<String>)> {
    let vault = Vault::open(path)?;

    // Debris such as .DS_Store does not make a folder "non-empty"; a directory
    // holding only that should still receive the full template.
    let has_content = vault
        .children(&RelPath::root())?
        .iter()
        .any(|child| !paths::is_junk(&child.name));

    // A folder already in use is the user's: no example notes appear in it.
    let (mode, created) = if has_content {
        (CreateMode::Registered, Vec::new())
    } else {
        (CreateMode::Scaffolded, template::scaffold_full(&vault)?.created())
    };

    registry::register(&vault)?;
    Ok((mode, created))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::ErrorCode;

    fn root_dir() -> (tempfile::TempDir, Utf8PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).unwrap();
        (dir, path)
    }

    fn create(root: &Utf8PathBuf, name: &str) -> Result<CreateVaultResponse> {
        create_vault(CreateVaultRequest {
            name: name.to_string(),
            root: root.clone(),
        })
    }

    #[test]
    fn creating_into_a_missing_target_scaffolds_the_full_template() {
        let (_tmp, root) = root_dir();
        let response = create(&root, "demo").unwrap();

        assert_eq!(response.mode, CreateMode::Scaffolded);
        assert_eq!(response.path, root.join("demo"));
        let vault = Vault::open(&response.path).unwrap();
        assert!(vault.is_file(&RelPath::parse("ideas/hello_world.md").unwrap()));
    }

    #[test]
    fn creating_into_an_existing_empty_folder_also_scaffolds_fully() {
        let (_tmp, root) = root_dir();
        std::fs::create_dir(root.join("demo")).unwrap();
        assert_eq!(create(&root, "demo").unwrap().mode, CreateMode::Scaffolded);
    }

    #[test]
    fn an_existing_folder_is_registered_and_left_untouched() {
        let (_tmp, root) = root_dir();
        let existing = root.join("existing-vault");
        // A hidden folder as well as an ordinary note: initialization must not
        // reach into whatever other tool owns the vault's configuration.
        std::fs::create_dir_all(existing.join(".config")).unwrap();
        std::fs::write(existing.join(".config/app.json"), b"{\"mine\":true}").unwrap();
        std::fs::write(existing.join("my_note.md"), b"my content").unwrap();

        let response = create(&root, "existing-vault").unwrap();
        assert_eq!(response.mode, CreateMode::Registered);
        assert!(response.created.is_empty());

        let vault = Vault::open(&existing).unwrap();
        // Existing content is untouched and no example notes appear.
        assert_eq!(
            vault.read(&RelPath::parse("my_note.md").unwrap()).unwrap(),
            b"my content"
        );
        assert_eq!(
            vault.read(&RelPath::parse(".config/app.json").unwrap()).unwrap(),
            b"{\"mine\":true}"
        );
        assert!(!vault.exists(&RelPath::parse("ideas").unwrap()));
        assert!(!vault.exists(&RelPath::parse("projects").unwrap()));
    }

    #[test]
    fn creating_twice_is_safe_and_changes_nothing_the_second_time() {
        let (_tmp, root) = root_dir();
        create(&root, "demo").unwrap();
        let again = create(&root, "demo").unwrap();

        // The vault now has content, so the second run only registers it.
        assert_eq!(again.mode, CreateMode::Registered);
        assert!(again.created.is_empty());
    }

    #[test]
    fn a_folder_holding_only_filesystem_debris_still_counts_as_empty() {
        let (_tmp, root) = root_dir();
        std::fs::create_dir(root.join("demo")).unwrap();
        std::fs::write(root.join("demo/.DS_Store"), b"junk").unwrap();

        assert_eq!(create(&root, "demo").unwrap().mode, CreateMode::Scaffolded);
    }

    #[test]
    fn the_name_must_be_one_folder_not_a_path() {
        let (_tmp, root) = root_dir();
        for name in ["", ".", "..", "a/b", "../escape", ".hidden", "a\\b"] {
            let err = create(&root, name).unwrap_err();
            assert_eq!(err.code, ErrorCode::InvalidInput, "accepted {name:?}");
        }
        assert!(!root.join("escape").exists());
    }

    #[test]
    fn a_missing_root_is_reported_as_not_found() {
        let (_tmp, root) = root_dir();
        let err = create_vault(CreateVaultRequest {
            name: "demo".to_string(),
            root: root.join("no-such-dir"),
        })
        .unwrap_err();
        assert_eq!(err.code, ErrorCode::NotFound);
    }

    #[test]
    fn an_existing_file_with_the_vault_name_is_a_conflict() {
        let (_tmp, root) = root_dir();
        std::fs::write(root.join("demo"), b"not a directory").unwrap();

        let err = create(&root, "demo").unwrap_err();
        assert_eq!(err.code, ErrorCode::AlreadyExists);
        assert_eq!(std::fs::read(root.join("demo")).unwrap(), b"not a directory");
    }
}

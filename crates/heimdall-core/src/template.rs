//! The default vault contents, embedded into the binary.
//!
//! `docs/vault-template/` is compiled in with `include_dir` so the CLI needs no
//! runtime assets (SPEC §7). Whatever the template directory holds is what a
//! new vault receives — editing the template changes new vaults without a code
//! change — minus filesystem debris, which is never vault content.

use include_dir::{include_dir, Dir as Embedded, DirEntry};

use crate::errors::Result;
use crate::paths::{self, RelPath};
use crate::storage::Vault;

static TEMPLATE: Embedded<'_> = include_dir!("$CARGO_MANIFEST_DIR/../../docs/vault-template");

/// What a scaffold actually wrote, for reporting and rollback.
#[derive(Debug, Default)]
pub struct Scaffold {
    pub created_files: Vec<RelPath>,
    pub created_dirs: Vec<RelPath>,
}

impl Scaffold {
    /// Paths written, as vault-relative strings, directories suffixed with `/`.
    pub fn created(&self) -> Vec<String> {
        let mut all: Vec<String> = self
            .created_dirs
            .iter()
            .map(|p| format!("{p}/"))
            .chain(self.created_files.iter().map(|p| p.to_string()))
            .collect();
        all.sort();
        all
    }

    /// Undo a failed scaffold. Only empty directories are removed (SPEC §7);
    /// files that landed are user data from that moment on.
    pub fn rollback_empty_dirs(&self, vault: &Vault) {
        for dir in self.created_dirs.iter().rev() {
            vault.remove_dir_if_empty(dir);
        }
    }
}

/// Write the complete template: `.obsidian/`, `aios/`, and the example notes.
pub fn scaffold_full(vault: &Vault) -> Result<Scaffold> {
    scaffold(vault, false)
}

/// Add only the managed `aios/` structure, leaving every existing note,
/// `.obsidian/`, and any other content untouched.
pub fn scaffold_aios_only(vault: &Vault) -> Result<Scaffold> {
    scaffold(vault, true)
}

fn scaffold(vault: &Vault, aios_only: bool) -> Result<Scaffold> {
    let mut scaffold = Scaffold::default();
    match write_entries(vault, &TEMPLATE, aios_only, &mut scaffold) {
        Ok(()) => {}
        Err(err) => {
            scaffold.rollback_empty_dirs(vault);
            return Err(err);
        }
    }

    // The managed container directories are created here rather than embedded:
    // an empty directory has nothing to include, and neither git nor
    // `include_dir` can carry one.
    for dir in paths::REQUIRED_DIRS {
        let path = RelPath::parse(dir)?;
        match vault.create_dir_all(&path) {
            Ok(created) => scaffold.created_dirs.extend(created),
            Err(err) => {
                scaffold.rollback_empty_dirs(vault);
                return Err(err);
            }
        }
    }
    Ok(scaffold)
}

fn write_entries(
    vault: &Vault,
    dir: &Embedded<'_>,
    aios_only: bool,
    scaffold: &mut Scaffold,
) -> Result<()> {
    for entry in dir.entries() {
        match entry {
            DirEntry::Dir(child) => {
                let Some(path) = template_path(child.path())? else {
                    continue;
                };
                if aios_only && !path.is_in_aios() {
                    continue;
                }
                scaffold.created_dirs.extend(vault.create_dir_all(&path)?);
                write_entries(vault, child, aios_only, scaffold)?;
            }
            DirEntry::File(file) => {
                let Some(path) = template_path(file.path())? else {
                    continue;
                };
                if aios_only && !path.is_in_aios() {
                    continue;
                }
                // Existing files are never overwritten (SPEC §7).
                if vault.exists(&path) {
                    continue;
                }
                scaffold.created_dirs.extend(vault.create_dir_all(&path.parent())?);
                vault.create_new(&path, file.contents())?;
                scaffold.created_files.push(path);
            }
        }
    }
    Ok(())
}

/// Convert an embedded path into a vault-relative path, or `None` if the entry
/// is filesystem debris that must not reach a user's vault.
fn template_path(path: &std::path::Path) -> Result<Option<RelPath>> {
    let raw = path.to_string_lossy().replace('\\', "/");
    let path = RelPath::parse(&raw)?;
    if path.components().any(paths::is_junk) {
        return Ok(None);
    }
    Ok(Some(path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use camino::Utf8PathBuf;

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
    fn the_template_is_embedded_including_the_obsidian_dotdir() {
        // include_dir must carry dot-directories; if it ever stops, new vaults
        // would silently lose their Obsidian settings.
        assert!(TEMPLATE.get_file(".obsidian/app.json").is_some());
        assert!(TEMPLATE.get_dir("aios").is_some());
    }

    #[test]
    fn a_full_scaffold_writes_the_template_and_every_managed_directory() {
        let (_tmp, vault) = temp_vault();
        let scaffold = scaffold_full(&vault).unwrap();

        vault.ensure_initialized().unwrap();
        assert!(vault.is_file(&rel(".obsidian/app.json")));
        assert!(vault.is_file(&rel("ideas/hello_world.md")));
        for dir in paths::REQUIRED_DIRS {
            assert!(vault.is_dir(&rel(dir)), "{dir} missing");
        }
        assert!(!scaffold.created_files.is_empty());
    }

    #[test]
    fn filesystem_debris_never_reaches_a_new_vault() {
        let (_tmp, vault) = temp_vault();
        scaffold_full(&vault).unwrap();
        assert!(!vault.exists(&rel(".DS_Store")));
        assert!(!vault.exists(&rel("aios/.DS_Store")));
        assert!(!vault.exists(&rel(".obsidian/snippets/.DS_Store")));
    }

    #[test]
    fn an_aios_only_scaffold_adds_no_notes_and_no_obsidian_settings() {
        let (_tmp, vault) = temp_vault();
        scaffold_aios_only(&vault).unwrap();

        vault.ensure_initialized().unwrap();
        assert!(!vault.exists(&rel(".obsidian")));
        assert!(!vault.exists(&rel("ideas")));
        assert!(!vault.exists(&rel("projects")));
    }

    #[test]
    fn scaffolding_never_overwrites_an_existing_file() {
        let (_tmp, vault) = temp_vault();
        vault.create_dir_all(&rel("ideas")).unwrap();
        vault
            .atomic_write(&rel("ideas/hello_world.md"), b"my own note")
            .unwrap();

        scaffold_full(&vault).unwrap();
        assert_eq!(vault.read(&rel("ideas/hello_world.md")).unwrap(), b"my own note");
    }

    #[test]
    fn scaffolding_is_idempotent() {
        let (_tmp, vault) = temp_vault();
        scaffold_full(&vault).unwrap();
        let second = scaffold_full(&vault).unwrap();

        assert!(second.created_files.is_empty());
        assert!(second.created_dirs.is_empty());
        vault.ensure_initialized().unwrap();
    }
}

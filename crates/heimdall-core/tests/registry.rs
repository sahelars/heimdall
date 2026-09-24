//! Finding a vault from a working directory (SPEC §7).
//!
//! The shell's `read`, `write`, `lock`, and `unlock` need no `--vault` when run
//! from inside a vault: the registry names the roots, and the nearest one above
//! the working directory is the vault.

use camino::{Utf8Path, Utf8PathBuf};
use heimdall_core::registry::{locate, register};
use heimdall_core::{ErrorCode, RelPath, Vault};

/// Each test gets its own data directory: the registry is one shared file, and
/// tests asserting "nothing is registered" must not see another test's vaults.
fn setup() -> (tempfile::TempDir, Utf8PathBuf, Utf8PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let base = Utf8PathBuf::from_path_buf(dir.path().canonicalize().unwrap()).unwrap();
    let data = base.join("data");
    let vaults = base.join("vaults");
    std::fs::create_dir_all(&vaults).unwrap();
    (dir, data, vaults)
}

fn make_vault(at: &Utf8Path, data: &Utf8Path) -> Vault {
    std::fs::create_dir_all(at).unwrap();
    let vault = Vault::open_with_data_dir(at, data).unwrap();
    register(&vault).unwrap();
    vault
}

#[test]
fn a_subfolder_finds_its_vault_and_knows_where_it_is_inside() {
    let (_tmp, data, vaults) = setup();
    let root = vaults.join("notes");
    make_vault(&root, &data);
    std::fs::create_dir_all(root.join("projects/deep")).unwrap();

    let located = locate(&root.join("projects/deep"), &data).unwrap();
    assert_eq!(located.vault.root(), root);
    assert_eq!(located.cwd, RelPath::parse("projects/deep").unwrap());

    let at_root = locate(&root, &data).unwrap();
    assert!(at_root.cwd.is_root());
}

#[test]
fn the_nearest_registered_root_wins_for_a_nested_vault() {
    let (_tmp, data, vaults) = setup();
    let outer = vaults.join("outer");
    let inner = outer.join("inner");
    make_vault(&outer, &data);
    make_vault(&inner, &data);

    assert_eq!(locate(&inner, &data).unwrap().vault.root(), inner);
    assert_eq!(locate(&outer, &data).unwrap().vault.root(), outer);
}

#[test]
fn a_folder_in_no_vault_says_how_to_fix_it() {
    let (_tmp, data, vaults) = setup();
    let err = locate(&vaults, &data).unwrap_err();
    assert_eq!(err.code, ErrorCode::NotInitialized);
    assert!(err.message.contains("--vault"), "{}", err.message);
}

#[test]
fn a_sibling_with_a_longer_name_is_not_inside() {
    let (_tmp, data, vaults) = setup();
    make_vault(&vaults.join("notes"), &data);
    std::fs::create_dir_all(vaults.join("notes2")).unwrap();
    assert!(locate(&vaults.join("notes2"), &data).is_err());
}

#[test]
fn registering_twice_records_the_vault_once() {
    let (_tmp, data, vaults) = setup();
    let vault = make_vault(&vaults.join("notes"), &data);
    register(&vault).unwrap();

    let stored: serde_json::Value =
        serde_json::from_slice(&std::fs::read(data.join("vaults.json")).unwrap()).unwrap();
    assert_eq!(stored["vaults"].as_array().unwrap().len(), 1);
}

#[cfg(unix)]
#[test]
fn a_symlinked_working_directory_finds_the_real_vault() {
    let (_tmp, data, vaults) = setup();
    let root = vaults.join("notes");
    make_vault(&root, &data);
    std::fs::create_dir_all(root.join("projects")).unwrap();
    let link = vaults.join("shortcut");
    std::os::unix::fs::symlink(root.join("projects"), &link).unwrap();

    let located = locate(&link, &data).unwrap();
    assert_eq!(located.vault.root(), root);
    assert_eq!(located.cwd, RelPath::parse("projects").unwrap());
}

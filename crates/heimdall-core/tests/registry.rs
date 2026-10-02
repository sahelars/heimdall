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

// --- Forgetting vaults whose folders are gone -------------------------------

mod forgetting {
    use super::*;
    use heimdall_core::agents;
    use heimdall_core::commands::{lock, LockRequest};
    use heimdall_core::registry::{forget_missing, list};
    use heimdall_core::storage::vault_is_gone;

    fn note(vault: &Vault) {
        std::fs::write(vault.root().join("a.md").as_std_path(), b"# a\n").unwrap();
    }

    #[test]
    fn a_deleted_vault_is_forgotten_everywhere_heimdall_kept_it() {
        let (_tmp, data, vaults) = setup();
        let kept = make_vault(&vaults.join("kept"), &data);
        let gone = make_vault(&vaults.join("gone"), &data);
        agents::share(&kept, None).unwrap();
        agents::share(&gone, None).unwrap();
        // Give it lock rules and a write lock, the two per-vault files.
        lock(&gone, LockRequest::default()).unwrap();
        let rules = gone.state_path();
        let write_lock = gone.lock_path();
        assert!(rules.exists() && write_lock.exists());
        let gone_root = gone.root().to_owned();
        drop(gone);

        std::fs::remove_dir_all(gone_root.as_std_path()).unwrap();
        let forgotten = forget_missing(&data).unwrap();

        assert_eq!(forgotten, [gone_root.clone()]);
        assert_eq!(list(&data).unwrap(), [kept.root().to_owned()]);
        let shared: Vec<_> = agents::list(&data).unwrap().into_iter().map(|v| v.path).collect();
        assert_eq!(shared, [kept.root().to_owned()]);
        let stored = std::fs::read_to_string(data.join("agents.json")).unwrap();
        assert!(!stored.contains(gone_root.as_str()), "{stored}");
        assert!(!rules.exists(), "its lock rules are left behind");
        assert!(!write_lock.exists(), "its write lock is left behind");
    }

    #[test]
    fn forgetting_twice_forgets_nothing_more_and_leaves_live_vaults_alone() {
        let (_tmp, data, vaults) = setup();
        let kept = make_vault(&vaults.join("kept"), &data);
        note(&kept);
        lock(&kept, LockRequest::default()).unwrap();
        let rules = std::fs::read(kept.state_path()).unwrap();

        assert!(forget_missing(&data).unwrap().is_empty());
        assert!(forget_missing(&data).unwrap().is_empty());
        assert_eq!(list(&data).unwrap(), [kept.root().to_owned()]);
        assert_eq!(std::fs::read(kept.state_path()).unwrap(), rules);
        // Its note is still locked: nothing about a live vault is touched.
        let _ = std::process::Command::new("chflags")
            .args(["-R", "nouchg"])
            .arg(kept.root().as_std_path())
            .status();
    }

    #[cfg(unix)]
    #[test]
    fn a_vault_this_process_may_not_enter_is_not_forgotten() {
        // What a macOS privacy refusal looks like from here: the folder is
        // there, and looking at it is refused. That is not "deleted".
        use std::os::unix::fs::PermissionsExt;
        let (_tmp, data, vaults) = setup();
        let fenced = vaults.join("fence");
        make_vault(&fenced.join("inside"), &data);

        std::fs::set_permissions(&fenced, std::fs::Permissions::from_mode(0o000)).unwrap();
        let outcome = forget_missing(&data);
        std::fs::set_permissions(&fenced, std::fs::Permissions::from_mode(0o755)).unwrap();

        assert!(outcome.unwrap().is_empty());
        assert_eq!(list(&data).unwrap(), [fenced.join("inside")]);
    }

    #[test]
    fn a_vault_on_an_unplugged_drive_is_not_forgotten() {
        assert!(!vault_is_gone(Utf8Path::new(
            "/Volumes/heimdall-no-such-drive-7f3a/notes"
        )));
        // A missing folder on a drive that is there is gone, like anywhere else.
        assert!(vault_is_gone(Utf8Path::new("/tmp/heimdall-no-such-vault-7f3a")));
    }
}

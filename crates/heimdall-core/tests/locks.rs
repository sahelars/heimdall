//! Note locks: a locked path is read-only for everyone (SPEC §6, §10).
//!
//! Every operation that changes a note's bytes or its place is refused with
//! `LOCKED` and names the rule responsible; reads are unaffected; and nothing
//! about a lock is ever written into the vault.

use camino::Utf8PathBuf;
use heimdall_core::commands::{
    create_folder, delete_path, lock, move_path, read, unlock, write, CreateFolderRequest,
    DeletePathRequest, LockRequest, MovePathRequest, ReadRequest, WriteRequest,
};
use heimdall_core::{template, Error, ErrorCode, Revision, Vault};

fn vault() -> (tempfile::TempDir, Vault) {
    let dir = tempfile::tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).unwrap();
    let vault = Vault::open_with_data_dir(&root, &test_data_dir()).unwrap();
    template::scaffold_full(&vault).unwrap();
    (dir, vault)
}

fn at(path: &str) -> LockRequest {
    LockRequest {
        path: Some(path.to_string()),
    }
}

fn revision(vault: &Vault, path: &str) -> Revision {
    read(
        vault,
        ReadRequest {
            path: Some(path.to_string()),
            ..Default::default()
        },
    )
    .unwrap()
    .document
    .unwrap()
    .revision
}

fn replace(vault: &Vault, path: &str, content: &str) -> Result<(), Error> {
    let expected = revision(vault, path);
    write(
        vault,
        WriteRequest {
            path: path.to_string(),
            content: content.to_string(),
            expected_revision: Some(Some(expected)),
        },
    )
    .map(|_| ())
}

fn create(vault: &Vault, path: &str) -> Result<(), Error> {
    write(
        vault,
        WriteRequest {
            path: path.to_string(),
            content: "# New\n".to_string(),
            expected_revision: None,
        },
    )
    .map(|_| ())
}

fn is_locked(vault: &Vault, path: &str) -> bool {
    read(
        vault,
        ReadRequest {
            path: Some(path.to_string()),
            ..Default::default()
        },
    )
    .unwrap()
    .locked
}

fn assert_locked_by(result: Result<(), Error>, locked_at: &str) {
    let err = result.expect_err("a locked path was changed");
    assert_eq!(err.code, ErrorCode::Locked, "{}", err.message);
    assert_eq!(err.details["locked_at"], locked_at);
}

#[test]
fn a_locked_note_reads_but_does_not_write() {
    let (_dir, vault) = vault();
    let response = lock(&vault, at("ideas/hello_world.md")).unwrap();
    assert!(response.locked && response.changed);

    assert!(is_locked(&vault, "ideas/hello_world.md"));
    assert_locked_by(replace(&vault, "ideas/hello_world.md", "x\n"), "ideas/hello_world.md");

    unlock(&vault, at("ideas/hello_world.md")).unwrap();
    replace(&vault, "ideas/hello_world.md", "x\n").unwrap();
}

#[test]
fn locking_a_folder_locks_every_note_in_it_and_refuses_new_ones() {
    let (_dir, vault) = vault();
    create_folder(&vault, CreateFolderRequest { path: "ideas/deep".into() }).unwrap();
    create(&vault, "ideas/deep/note.md").unwrap();
    lock(&vault, at("ideas")).unwrap();

    assert_locked_by(replace(&vault, "ideas/deep/note.md", "x\n"), "ideas");
    assert_locked_by(create(&vault, "ideas/deep/new.md"), "ideas");
    assert_locked_by(
        create_folder(&vault, CreateFolderRequest { path: "ideas/other".into() }).map(|_| ()),
        "ideas",
    );
    // Another folder is untouched.
    replace(&vault, "projects/my_project.md", "x\n").unwrap();
}

#[test]
fn one_note_can_be_unlocked_inside_a_locked_folder_until_the_folder_is_locked_again() {
    let (_dir, vault) = vault();
    create(&vault, "ideas/draft.md").unwrap();
    lock(&vault, at("ideas")).unwrap();
    unlock(&vault, at("ideas/draft.md")).unwrap();

    replace(&vault, "ideas/draft.md", "editable\n").unwrap();
    assert!(is_locked(&vault, "ideas/hello_world.md"));

    // Locking the folder again means all of it.
    lock(&vault, at("ideas")).unwrap();
    assert_locked_by(replace(&vault, "ideas/draft.md", "x\n"), "ideas");
}

#[test]
fn unlocking_a_folder_unlocks_the_notes_given_their_own_lock() {
    let (_dir, vault) = vault();
    lock(&vault, at("ideas/hello_world.md")).unwrap();
    unlock(&vault, at("ideas")).unwrap();
    replace(&vault, "ideas/hello_world.md", "x\n").unwrap();
}

#[test]
fn locking_the_vault_root_locks_everything() {
    let (_dir, vault) = vault();
    lock(&vault, LockRequest::default()).unwrap();

    assert!(is_locked(&vault, ""));
    assert_locked_by(replace(&vault, "projects/my_project.md", "x\n"), "");
    assert_locked_by(create(&vault, "top.md"), "");

    unlock(&vault, LockRequest::default()).unwrap();
    create(&vault, "top.md").unwrap();
}

#[test]
fn a_lock_fixes_a_notes_place_as_well_as_its_bytes() {
    let (_dir, vault) = vault();
    lock(&vault, at("ideas/hello_world.md")).unwrap();

    let moving = move_path(
        &vault,
        MovePathRequest {
            from: "ideas/hello_world.md".into(),
            to: "ideas/renamed.md".into(),
        },
    );
    assert_locked_by(moving.map(|_| ()), "ideas/hello_world.md");

    // The folder around a locked note cannot move or go to the trash either,
    // because it would take the note with it.
    let folder = move_path(
        &vault,
        MovePathRequest {
            from: "ideas".into(),
            to: "thoughts".into(),
        },
    );
    assert_locked_by(folder.map(|_| ()), "ideas/hello_world.md");
    let trashed = delete_path(
        &vault,
        DeletePathRequest {
            path: "ideas".into(),
            expected_revision: None,
        },
    );
    assert_locked_by(trashed.map(|_| ()), "ideas/hello_world.md");
}

#[test]
fn nothing_moves_into_or_out_of_a_locked_folder() {
    let (_dir, vault) = vault();
    lock(&vault, at("projects")).unwrap();

    let into = move_path(
        &vault,
        MovePathRequest {
            from: "ideas/hello_world.md".into(),
            to: "projects/hello_world.md".into(),
        },
    );
    assert_locked_by(into.map(|_| ()), "projects");

    let out = move_path(
        &vault,
        MovePathRequest {
            from: "projects/my_project.md".into(),
            to: "ideas/my_project.md".into(),
        },
    );
    assert_locked_by(out.map(|_| ()), "projects");
}

#[test]
fn a_trashed_note_takes_its_lock_rule_with_it() {
    let (_dir, vault) = vault();
    create(&vault, "ideas/temp.md").unwrap();
    lock(&vault, at("ideas/temp.md")).unwrap();
    unlock(&vault, at("ideas/temp.md")).unwrap();
    delete_path(
        &vault,
        DeletePathRequest {
            path: "ideas/temp.md".into(),
            expected_revision: None,
        },
    )
    .unwrap();

    // A new note at the same path starts out writable.
    create(&vault, "ideas/temp.md").unwrap();
    assert!(!is_locked(&vault, "ideas/temp.md"));
}

#[test]
fn locking_twice_is_not_a_change() {
    let (_dir, vault) = vault();
    assert!(lock(&vault, at("ideas")).unwrap().changed);
    assert!(!lock(&vault, at("ideas")).unwrap().changed);
    assert!(!lock(&vault, at("ideas/hello_world.md")).unwrap().changed);
}

#[test]
fn only_folders_and_notes_that_exist_can_be_locked() {
    let (dir, vault) = vault();
    std::fs::write(dir.path().join("image.png"), b"x").unwrap();

    assert_eq!(lock(&vault, at("nowhere.md")).unwrap_err().code, ErrorCode::NotFound);
    assert_eq!(lock(&vault, at("image.png")).unwrap_err().code, ErrorCode::InvalidInput);
    assert_eq!(lock(&vault, at(".trash")).unwrap_err().code, ErrorCode::InvalidInput);
}

#[test]
fn locking_writes_nothing_into_the_vault() {
    let (dir, vault) = vault();
    let before = snapshot(dir.path());
    lock(&vault, LockRequest::default()).unwrap();
    unlock(&vault, at("ideas")).unwrap();
    assert_eq!(snapshot(dir.path()), before);
    assert!(vault.state_path().exists(), "the rules live outside the vault");
}

/// Every path under a directory, hidden ones included.
fn snapshot(root: &std::path::Path) -> Vec<String> {
    let mut paths = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let entry = entry.unwrap();
            paths.push(entry.path().display().to_string());
            if entry.file_type().unwrap().is_dir() {
                stack.push(entry.path());
            }
        }
    }
    paths.sort();
    paths
}

/// Where these tests keep Heimdall's application data (write locks, lock rules).
///
/// Outside the vault, as production does, but under the system temp directory
/// rather than the real application-data one. Per-vault files are named by a
/// hash of the vault's path and every vault here is a fresh temp directory, so
/// sharing one directory cannot collide.
fn test_data_dir() -> camino::Utf8PathBuf {
    camino::Utf8PathBuf::from_path_buf(std::env::temp_dir())
        .expect("temp dir is UTF-8")
        .join("heimdall-test-data")
}

//! Note locks: a locked path is read-only for everyone (SPEC §6, §10).
//!
//! Every operation that changes a note's bytes or its place is refused with
//! `LOCKED` and names the rule responsible; reads are unaffected; no other
//! spelling of a locked path gets past it; on macOS the operating system
//! refuses the write too; and unlocking needs a person.

use camino::Utf8PathBuf;
use heimdall_core::commands::{
    create_folder, delete_path, link_graph, lock, move_path, read, relink, unlock, write,
    CreateFolderRequest, DeletePathRequest, LinkGraphRequest, LockRequest, MovePathRequest,
    Presence, ReadRequest, RelinkRequest, WriteRequest,
};
use heimdall_core::{template, Error, ErrorCode, Result, Revision, Vault};

/// A temporary vault whose locked notes are made deletable again on drop.
///
/// A note a lock flagged immutable cannot be removed, so `TempDir` would
/// silently leave every one of them behind in the system temp directory.
struct TempVault(tempfile::TempDir);

impl TempVault {
    fn path(&self) -> &std::path::Path {
        self.0.path()
    }
}

impl Drop for TempVault {
    fn drop(&mut self) {
        let _ = std::process::Command::new("chflags")
            .args(["-R", "nouchg"])
            .arg(self.0.path())
            .status();
    }
}

fn vault() -> (TempVault, Vault) {
    let dir = tempfile::tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).unwrap();
    let vault = Vault::open_with_data_dir(&root, &test_data_dir()).unwrap();
    template::scaffold_full(&vault).unwrap();
    (TempVault(dir), vault)
}

/// A person who said yes.
struct Confirmed;

impl Presence for Confirmed {
    fn confirm(&self, _action: &str) -> Result<()> {
        Ok(())
    }
}

/// A person who said no, recording what they were asked.
#[derive(Default)]
struct Refused(std::cell::RefCell<Vec<String>>);

impl Presence for Refused {
    fn confirm(&self, action: &str) -> Result<()> {
        self.0.borrow_mut().push(action.to_string());
        Err(Error::not_confirmed("cancelled", "cancelled"))
    }
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
            vault: None,
            path: Some(path.to_string()),
            ..Default::default()
        },
    )
    .unwrap()
    .document
    .unwrap()
    .revision
}

fn replace(vault: &Vault, path: &str, content: &str) -> Result<()> {
    let expected = revision(vault, path);
    write(
        vault,
        WriteRequest {
            vault: None,
            path: path.to_string(),
            content: content.to_string(),
            expected_revision: Some(Some(expected)),
        },
    )
    .map(|_| ())
}

fn create(vault: &Vault, path: &str) -> Result<()> {
    write(
        vault,
        WriteRequest {
            vault: None,
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
            vault: None,
            path: Some(path.to_string()),
            ..Default::default()
        },
    )
    .unwrap()
    .locked
}

fn assert_locked_by(result: Result<()>, locked_at: &str) {
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

    unlock(&vault, at("ideas/hello_world.md"), &Confirmed).unwrap();
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
    unlock(&vault, at("ideas/draft.md"), &Confirmed).unwrap();

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
    unlock(&vault, at("ideas"), &Confirmed).unwrap();
    replace(&vault, "ideas/hello_world.md", "x\n").unwrap();
}

#[test]
fn locking_the_vault_root_locks_everything() {
    let (_dir, vault) = vault();
    lock(&vault, LockRequest::default()).unwrap();

    assert!(is_locked(&vault, ""));
    assert_locked_by(replace(&vault, "projects/my_project.md", "x\n"), "");
    assert_locked_by(create(&vault, "top.md"), "");

    unlock(&vault, LockRequest::default(), &Confirmed).unwrap();
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
    unlock(&vault, at("ideas/temp.md"), &Confirmed).unwrap();
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
    unlock(&vault, at("ideas"), &Confirmed).unwrap();
    assert_eq!(snapshot(dir.path()), before);
    assert!(vault.state_path().exists(), "the rules live outside the vault");
}

// --- Other spellings of a locked path ----------------------------------------

/// Whether this filesystem treats `A` and `a` as one name — APFS and HFS+ do
/// by default. Case tests mean nothing on one that does not.
fn case_insensitive(vault: &TempVault) -> bool {
    std::fs::write(vault.path().join("probe-case.md"), b"").unwrap();
    let insensitive = vault.path().join("PROBE-CASE.md").exists();
    std::fs::remove_file(vault.path().join("probe-case.md")).unwrap();
    insensitive
}

#[test]
fn no_other_casing_of_a_locked_path_gets_past_the_lock() {
    let (dir, vault) = vault();
    if !case_insensitive(&dir) {
        return;
    }
    lock(&vault, at("ideas/hello_world.md")).unwrap();
    lock(&vault, at("projects")).unwrap();

    // The note itself, under a spelling no rule was ever written for.
    let shouted = write(
        &vault,
        WriteRequest {
            vault: None,
            path: "Ideas/Hello_World.md".into(),
            content: "x\n".into(),
            expected_revision: Some(Some(revision(&vault, "ideas/hello_world.md"))),
        },
    );
    assert_locked_by(shouted.map(|_| ()), "ideas/hello_world.md");
    // A "new" note that the filesystem would open as the locked one.
    assert_eq!(
        create(&vault, "IDEAS/HELLO_WORLD.md").unwrap_err().code,
        ErrorCode::Locked
    );
    // A new note in the locked folder, spelled differently.
    assert_locked_by(create(&vault, "Projects/new.md"), "projects");
    assert_locked_by(
        create_folder(&vault, CreateFolderRequest { path: "PROJECTS/sub".into() }).map(|_| ()),
        "projects",
    );
    let moved = move_path(
        &vault,
        MovePathRequest {
            from: "Ideas/Hello_World.md".into(),
            to: "ideas/elsewhere.md".into(),
        },
    );
    assert_locked_by(moved.map(|_| ()), "ideas/hello_world.md");
    let trashed = delete_path(
        &vault,
        DeletePathRequest {
            path: "Projects".into(),
            expected_revision: None,
        },
    );
    assert_locked_by(trashed.map(|_| ()), "projects");
    assert!(is_locked(&vault, "IDEAS/hello_WORLD.md"));
}

#[test]
fn a_rule_is_stored_under_the_spelling_on_disk() {
    let (dir, vault) = vault();
    if !case_insensitive(&dir) {
        return;
    }
    let response = lock(&vault, at("Projects")).unwrap();
    assert_eq!(response.path, "projects");
    let stored = std::fs::read_to_string(vault.state_path()).unwrap();
    assert!(stored.contains("\"projects\""), "{stored}");
    assert_locked_by(replace(&vault, "projects/my_project.md", "x\n"), "projects");
}

#[test]
fn a_rule_written_in_another_casing_still_locks() {
    // Rules written before they were stored by real spelling, or by hand.
    let (dir, vault) = vault();
    if !case_insensitive(&dir) {
        return;
    }
    std::fs::create_dir_all(vault.state_path().parent().unwrap()).unwrap();
    std::fs::write(vault.state_path(), r#"{"rules":{"PROJECTS":true}}"#).unwrap();
    assert_locked_by(replace(&vault, "projects/my_project.md", "x\n"), "projects");
}

#[test]
fn a_decomposed_spelling_of_a_locked_name_gets_no_further() {
    let (dir, vault) = vault();
    if !case_insensitive(&dir) {
        return;
    }
    create_folder(&vault, CreateFolderRequest { path: "caf\u{e9}".into() }).unwrap();
    create(&vault, "caf\u{e9}/menu.md").unwrap();
    lock(&vault, at("caf\u{e9}")).unwrap();
    // "e" followed by a combining acute accent: the same name to APFS.
    assert_eq!(
        create(&vault, "cafe\u{301}/new.md").unwrap_err().code,
        ErrorCode::Locked
    );
}

#[cfg(unix)]
#[test]
fn a_symlinked_folder_inside_the_vault_is_held_to_the_folder_it_names() {
    let (dir, vault) = vault();
    std::os::unix::fs::symlink("projects", dir.path().join("alias")).unwrap();
    lock(&vault, at("projects")).unwrap();

    assert_locked_by(replace(&vault, "alias/my_project.md", "x\n"), "projects");
    assert_locked_by(create(&vault, "alias/new.md"), "projects");
    let into = move_path(
        &vault,
        MovePathRequest {
            from: "ideas/hello_world.md".into(),
            to: "alias/hello_world.md".into(),
        },
    );
    assert_locked_by(into.map(|_| ()), "projects");

    let through = read(
        &vault,
        ReadRequest {
            vault: None,
            path: Some("alias/my_project.md".into()),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(through.locked);
    assert_eq!(through.locked_at.as_deref(), Some("projects"));

    let listing = read(
        &vault,
        ReadRequest {
            vault: None,
            path: Some("alias".into()),
            ..Default::default()
        },
    )
    .unwrap()
    .listing
    .unwrap();
    assert!(listing.entries.iter().all(|entry| entry.locked), "{listing:?}");

    let graph = link_graph(&vault, LinkGraphRequest::default()).unwrap();
    let aliased: Vec<_> = graph.nodes.iter().filter(|n| n.path.starts_with("alias/")).collect();
    assert!(!aliased.is_empty());
    assert!(aliased.iter().all(|node| node.locked));
}

#[cfg(unix)]
#[test]
fn relink_never_rewrites_a_locked_note_through_an_alias() {
    let (dir, vault) = vault();
    write(
        &vault,
        WriteRequest {
            vault: None,
            path: "projects/links.md".into(),
            content: "[[ideas/hello_world]]\n".into(),
            expected_revision: None,
        },
    )
    .unwrap();
    std::os::unix::fs::symlink("projects", dir.path().join("alias")).unwrap();
    lock(&vault, at("projects/links.md")).unwrap();
    move_path(
        &vault,
        MovePathRequest {
            from: "ideas/hello_world.md".into(),
            to: "ideas/renamed.md".into(),
        },
    )
    .unwrap();

    let outcome = relink(
        &vault,
        RelinkRequest {
            from: "ideas/hello_world.md".into(),
            to: "ideas/renamed.md".into(),
            dry_run: false,
        },
    )
    .unwrap();
    assert!(outcome.updated.is_empty(), "{outcome:?}");
    assert!(outcome.locked.iter().any(|p| p == "alias/links.md"), "{outcome:?}");
    assert_eq!(
        std::fs::read_to_string(dir.path().join("projects/links.md")).unwrap(),
        "[[ideas/hello_world]]\n"
    );
}

#[test]
fn a_lock_rule_that_cannot_be_read_locks_the_whole_vault() {
    let (_dir, vault) = vault();
    std::fs::create_dir_all(vault.state_path().parent().unwrap()).unwrap();
    std::fs::write(vault.state_path(), r#"{"rules":{"../escape":true,"/abs":false}}"#).unwrap();
    assert_locked_by(replace(&vault, "projects/my_project.md", "x\n"), "");
}

// --- Unlocking needs a person -----------------------------------------------

#[test]
fn an_unlock_nobody_confirmed_changes_nothing() {
    let (_dir, vault) = vault();
    lock(&vault, at("ideas")).unwrap();
    let rules_before = std::fs::read(vault.state_path()).unwrap();

    let refused = Refused::default();
    let err = unlock(&vault, at("ideas"), &refused).unwrap_err();
    assert_eq!(err.code, ErrorCode::NotConfirmed);
    assert_eq!(err.details["reason"], "cancelled");

    // The person was told what they were being asked, and where.
    let asked = refused.0.borrow();
    assert_eq!(asked.len(), 1);
    assert!(asked[0].contains("unlock \"ideas\""), "{}", asked[0]);
    assert!(asked[0].contains(vault.name()), "{}", asked[0]);

    assert_eq!(std::fs::read(vault.state_path()).unwrap(), rules_before);
    assert_locked_by(replace(&vault, "ideas/hello_world.md", "x\n"), "ideas");
}

#[test]
fn locking_asks_no_one() {
    // `lock` takes no `Presence` at all: making something read-only widens
    // nothing, and it must never be held up by a prompt.
    let (_dir, vault) = vault();
    lock(&vault, at("ideas")).unwrap();
}

#[test]
fn the_question_is_asked_only_about_something_that_can_be_unlocked() {
    // A typo is reported as a typo; nobody is asked to approve it first.
    let (_dir, vault) = vault();
    let refused = Refused::default();
    assert_eq!(
        unlock(&vault, at("nowhere.md"), &refused).unwrap_err().code,
        ErrorCode::NotFound
    );
    assert!(refused.0.borrow().is_empty());
}

// --- The operating system enforces it too (macOS) ---------------------------

#[cfg(target_os = "macos")]
mod file_flags {
    use super::*;
    use heimdall_core::commands::Protection;
    use std::os::macos::fs::MetadataExt;

    const UF_IMMUTABLE: u32 = 0x2;

    fn flagged(dir: &TempVault, path: &str) -> bool {
        std::fs::metadata(dir.path().join(path)).unwrap().st_flags() & UF_IMMUTABLE != 0
    }

    #[test]
    fn a_locked_note_cannot_be_written_by_anything() {
        let (dir, vault) = vault();
        let response = lock(&vault, at("ideas/hello_world.md")).unwrap();
        assert_eq!(response.protection, Protection::Complete);
        assert!(flagged(&dir, "ideas/hello_world.md"));

        // Not by a plain write, a replace-by-rename, or a delete — which is
        // what an editor, `sed -i`, `apply_patch`, and `rm` do.
        let target = dir.path().join("ideas/hello_world.md");
        assert!(std::fs::write(&target, b"x").is_err());
        std::fs::write(dir.path().join("ideas/tmp"), b"x").unwrap();
        assert!(std::fs::rename(dir.path().join("ideas/tmp"), &target).is_err());
        assert!(std::fs::remove_file(&target).is_err());

        unlock(&vault, at("ideas/hello_world.md"), &Confirmed).unwrap();
        assert!(!flagged(&dir, "ideas/hello_world.md"));
        std::fs::write(&target, b"x").unwrap();
    }

    #[test]
    fn a_folder_lock_flags_every_note_beneath_it_and_nothing_else() {
        let (dir, vault) = vault();
        create_folder(&vault, CreateFolderRequest { path: "ideas/deep".into() }).unwrap();
        create(&vault, "ideas/deep/note.md").unwrap();
        std::fs::write(dir.path().join("ideas/picture.png"), b"x").unwrap();
        std::fs::create_dir_all(dir.path().join("ideas/.tool")).unwrap();
        std::fs::write(dir.path().join("ideas/.tool/state.md"), b"x").unwrap();

        lock(&vault, at("ideas")).unwrap();
        assert!(flagged(&dir, "ideas/hello_world.md"));
        assert!(flagged(&dir, "ideas/deep/note.md"));
        assert!(!flagged(&dir, "ideas/picture.png"));
        assert!(!flagged(&dir, "ideas/.tool/state.md"));
        assert!(!flagged(&dir, "ideas"), "folders are never flagged");
        assert!(!flagged(&dir, "projects/my_project.md"));

        // A note unlocked inside the locked folder stays writable, temp file
        // and rename included, because the folder itself carries no flag.
        unlock(&vault, at("ideas/deep/note.md"), &Confirmed).unwrap();
        assert!(!flagged(&dir, "ideas/deep/note.md"));
        replace(&vault, "ideas/deep/note.md", "edited\n").unwrap();

        unlock(&vault, at("ideas"), &Confirmed).unwrap();
        assert!(!flagged(&dir, "ideas/hello_world.md"));
    }

    #[test]
    fn a_flagged_note_stays_locked_when_the_rules_are_gone() {
        // Deleting the rules file, or pointing Heimdall at another data
        // directory, must not unlock anything.
        let (dir, vault) = vault();
        lock(&vault, at("ideas/hello_world.md")).unwrap();
        std::fs::remove_file(vault.state_path()).unwrap();
        assert_locked_by(replace(&vault, "ideas/hello_world.md", "x\n"), "ideas/hello_world.md");

        let root = Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        let data = Utf8PathBuf::from_path_buf(elsewhere.path().to_path_buf()).unwrap();
        let other = Vault::open_with_data_dir(&root, &data).unwrap();
        assert_locked_by(replace(&other, "ideas/hello_world.md", "x\n"), "ideas/hello_world.md");
        assert!(is_locked(&other, "ideas/hello_world.md"));

        // Unlocking — with a person's say-so — is what lifts it.
        unlock(&vault, at("ideas/hello_world.md"), &Confirmed).unwrap();
        replace(&vault, "ideas/hello_world.md", "x\n").unwrap();
    }

    #[test]
    fn a_refused_unlock_leaves_the_flag_in_place() {
        let (dir, vault) = vault();
        lock(&vault, at("ideas")).unwrap();
        let _ = unlock(&vault, at("ideas"), &Refused::default());
        assert!(flagged(&dir, "ideas/hello_world.md"));
    }

    #[test]
    fn locking_never_lifts_a_flag() {
        // A note someone made immutable another way (Finder's "Locked") is
        // left alone by a lock elsewhere that does not cover it.
        let (dir, vault) = vault();
        lock(&vault, at("ideas/hello_world.md")).unwrap();
        std::fs::remove_file(vault.state_path()).unwrap();
        lock(&vault, at("projects")).unwrap();
        assert!(flagged(&dir, "ideas/hello_world.md"));
    }
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

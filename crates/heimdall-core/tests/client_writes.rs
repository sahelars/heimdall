//! The desktop client's write surface (SPEC §15, §17).
//!
//! These are the operations a human performs on their own vault through
//! Heimdall's own application: saving a note, making a folder, renaming, and
//! deleting. They are shell commands the desktop calls, never MCP tools, and
//! they inherit the same revision discipline every other write has.

use camino::Utf8PathBuf;
use heimdall_core::commands::{
    create_folder, delete_path, list_documents, move_path, read_documents, write_document,
    CreateFolderRequest, DeletePathRequest, DocumentSelection, ListDocumentsRequest,
    MovePathRequest, ReadDocumentsRequest, WriteDocumentRequest,
};
use heimdall_core::errors::ErrorCode;
use heimdall_core::{template, Revision, Vault};

fn vault() -> (tempfile::TempDir, Vault) {
    let dir = tempfile::tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).unwrap();
    let vault = Vault::open(&root).unwrap();
    template::scaffold_full(&vault).unwrap();
    (dir, vault)
}

/// Create a note, returning the revision the vault stored it at.
fn create(vault: &Vault, path: &str, content: &str) -> Revision {
    write_document(
        vault,
        WriteDocumentRequest {
            path: path.to_string(),
            content: content.to_string(),
            expected_revision: Some(None),
        },
    )
    .unwrap()
    .new_revision
}

fn read_back(vault: &Vault, path: &str) -> String {
    let response = read_documents(
        vault,
        ReadDocumentsRequest {
            documents: vec![DocumentSelection {
                path: path.to_string(),
                start_line: None,
                max_lines: None,
            }],
            max_total_bytes: None,
        },
    )
    .unwrap();
    response.documents[0]
        .result
        .as_ref()
        .expect("the document was returned")
        .content
        .clone()
}

/// Every path a recursive listing reports, at full depth.
fn listing(vault: &Vault) -> Vec<String> {
    list_documents(
        vault,
        ListDocumentsRequest {
            recursive: true,
            max_depth: Some(16),
            limit: Some(200),
            ..Default::default()
        },
    )
    .unwrap()
    .entries
    .into_iter()
    .map(|entry| entry.path)
    .collect()
}

#[test]
fn a_new_note_round_trips_through_the_read_surface() {
    let (_dir, vault) = vault();
    create(&vault, "ideas/new_note.md", "# New\n\nBody.\n");
    assert_eq!(read_back(&vault, "ideas/new_note.md"), "# New\n\nBody.\n");
}

#[test]
fn saving_requires_the_revision_the_note_was_read_at() {
    let (_dir, vault) = vault();
    let revision = create(&vault, "ideas/note.md", "one\n");

    let saved = write_document(
        &vault,
        WriteDocumentRequest {
            path: "ideas/note.md".into(),
            content: "two\n".into(),
            expected_revision: Some(Some(revision.clone())),
        },
    )
    .unwrap();
    assert!(!saved.created);
    assert_eq!(read_back(&vault, "ideas/note.md"), "two\n");

    // The first revision is now stale, and a second save with it must not win.
    let stale = write_document(
        &vault,
        WriteDocumentRequest {
            path: "ideas/note.md".into(),
            content: "three\n".into(),
            expected_revision: Some(Some(revision)),
        },
    )
    .unwrap_err();
    assert_eq!(stale.code, ErrorCode::RevisionConflict);
    // The conflict carries what the caller needs to recover without a re-read.
    assert_eq!(stale.details["current_revision"], saved.new_revision.as_str());
    assert_eq!(read_back(&vault, "ideas/note.md"), "two\n");
}

#[test]
fn omitting_the_revision_is_a_caller_mistake_rather_than_a_create() {
    let (_dir, vault) = vault();
    let error = write_document(
        &vault,
        WriteDocumentRequest {
            path: "ideas/note.md".into(),
            content: "body\n".into(),
            expected_revision: None,
        },
    )
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidInput);
    assert_eq!(error.details["parameter"], "expected_revision");
}

#[test]
fn creating_over_an_existing_note_is_a_conflict_not_an_overwrite() {
    let (_dir, vault) = vault();
    create(&vault, "ideas/note.md", "original\n");

    let error = write_document(
        &vault,
        WriteDocumentRequest {
            path: "ideas/note.md".into(),
            content: "replacement\n".into(),
            expected_revision: Some(None),
        },
    )
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::RevisionConflict);
    assert_eq!(read_back(&vault, "ideas/note.md"), "original\n");
}

#[test]
fn document_writes_cannot_reach_protected_content_in_any_casing() {
    let (_dir, vault) = vault();
    for path in ["aios/memories/memory.md", "AIOS/notes.md", "Aios/x.md"] {
        let error = write_document(
            &vault,
            WriteDocumentRequest {
                path: path.to_string(),
                content: "nope\n".into(),
                expected_revision: Some(None),
            },
        )
        .unwrap_err();
        // NOT_FOUND rather than a refusal: to an ordinary operation, the
        // protected tree is not there at all.
        assert_eq!(error.code, ErrorCode::NotFound, "{path}");
    }
}

#[test]
fn document_writes_cannot_reach_hidden_folders() {
    let (_dir, vault) = vault();
    // `.obsidian/` holds the user's Obsidian settings and `.trash/` holds what
    // they deleted. Neither is content, and neither is listable, so writing
    // there would create files nothing could ever show again.
    for path in [".obsidian/app.md", ".trash/ideas/note.md", ".git/config.md"] {
        let error = write_document(
            &vault,
            WriteDocumentRequest {
                path: path.to_string(),
                content: "nope\n".into(),
                expected_revision: Some(None),
            },
        )
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidInput, "{path}");
    }
}

#[test]
fn a_write_into_a_missing_folder_names_the_folder_rather_than_creating_it() {
    let (_dir, vault) = vault();
    let error = write_document(
        &vault,
        WriteDocumentRequest {
            path: "not_yet/note.md".into(),
            content: "body\n".into(),
            expected_revision: Some(None),
        },
    )
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::NotFound);
    assert_eq!(error.details["path"], "not_yet");
    // One mistyped path must not scatter directories through the vault.
    assert!(!listing(&vault).iter().any(|path| path == "not_yet"));
}

#[test]
fn create_folder_reports_only_the_directories_it_made() {
    let (_dir, vault) = vault();
    let response = create_folder(
        &vault,
        CreateFolderRequest {
            path: "projects/lens/deep".into(),
        },
    )
    .unwrap();
    // `projects/` already exists in the template, so it is not reported.
    assert_eq!(response.created, vec!["projects/lens", "projects/lens/deep"]);

    let again = create_folder(
        &vault,
        CreateFolderRequest {
            path: "projects/lens".into(),
        },
    )
    .unwrap_err();
    assert_eq!(again.code, ErrorCode::AlreadyExists);
}

#[test]
fn renaming_a_note_moves_its_content_and_leaves_nothing_behind() {
    let (_dir, vault) = vault();
    create(&vault, "ideas/before.md", "same bytes\n");

    let moved = move_path(
        &vault,
        MovePathRequest {
            from: "ideas/before.md".into(),
            to: "projects/after.md".into(),
        },
    )
    .unwrap();
    assert_eq!(moved.to, "projects/after.md");

    assert_eq!(read_back(&vault, "projects/after.md"), "same bytes\n");
    let paths = listing(&vault);
    assert!(!paths.iter().any(|path| path == "ideas/before.md"));
    assert!(paths.iter().any(|path| path == "projects/after.md"));
}

#[test]
fn a_move_cannot_cross_the_aios_boundary_in_either_direction() {
    let (_dir, vault) = vault();
    create(&vault, "ideas/note.md", "ordinary\n");

    // Into the protected tree: this would manufacture an entry with none of the
    // properties the entry contract guarantees.
    let inward = move_path(
        &vault,
        MovePathRequest {
            from: "ideas/note.md".into(),
            to: "aios/conversations/2026-08-16_10-00-00.md".into(),
        },
    )
    .unwrap_err();
    assert_eq!(inward.code, ErrorCode::InvalidInput);

    // Out of it: managed content would become a document nothing rewrites.
    let outward = move_path(
        &vault,
        MovePathRequest {
            from: "aios/memories/memory.md".into(),
            to: "ideas/memory.md".into(),
        },
    )
    .unwrap_err();
    assert_eq!(outward.code, ErrorCode::InvalidInput);

    assert_eq!(read_back(&vault, "ideas/note.md"), "ordinary\n");
}

#[test]
fn the_managed_structure_cannot_be_moved_or_deleted() {
    let (_dir, vault) = vault();
    for path in ["aios", "aios/memories", "aios/memories/memory.md"] {
        let moved = move_path(
            &vault,
            MovePathRequest {
                from: path.to_string(),
                to: "somewhere_else".into(),
            },
        )
        .unwrap_err();
        assert_eq!(moved.code, ErrorCode::InvalidInput, "moving {path}");

        let deleted = delete_path(
            &vault,
            DeletePathRequest {
                path: path.to_string(),
                expected_revision: None,
            },
        )
        .unwrap_err();
        assert_eq!(deleted.code, ErrorCode::InvalidInput, "deleting {path}");
    }
    // The point of all that: the vault still opens.
    vault.ensure_initialized().unwrap();
}

#[test]
fn a_move_onto_an_occupied_name_changes_nothing() {
    let (_dir, vault) = vault();
    create(&vault, "ideas/one.md", "first\n");
    create(&vault, "ideas/two.md", "second\n");

    let error = move_path(
        &vault,
        MovePathRequest {
            from: "ideas/one.md".into(),
            to: "ideas/two.md".into(),
        },
    )
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::AlreadyExists);
    assert_eq!(read_back(&vault, "ideas/one.md"), "first\n");
    assert_eq!(read_back(&vault, "ideas/two.md"), "second\n");
}

#[test]
fn a_note_cannot_be_renamed_out_of_markdown() {
    let (_dir, vault) = vault();
    create(&vault, "ideas/note.md", "body\n");

    // Every read operation reaches only `.md` files, so this rename would make
    // the note unreachable through Heimdall without ever deleting it.
    let error = move_path(
        &vault,
        MovePathRequest {
            from: "ideas/note.md".into(),
            to: "ideas/note.txt".into(),
        },
    )
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidInput);
    assert_eq!(error.details["parameter"], "to");
}

#[test]
fn deleting_a_note_moves_it_into_the_trash_rather_than_unlinking_it() {
    let (dir, vault) = vault();
    create(&vault, "projects/doomed.md", "still here\n");

    let deleted = delete_path(
        &vault,
        DeletePathRequest {
            path: "projects/doomed.md".into(),
            expected_revision: None,
        },
    )
    .unwrap();
    assert_eq!(deleted.trashed_to, ".trash/projects/doomed.md");

    // The bytes survive, mirrored under the original layout so a restore is
    // unambiguous.
    let trashed = dir.path().join(".trash/projects/doomed.md");
    assert_eq!(std::fs::read_to_string(trashed).unwrap(), "still here\n");

    // And it is gone from every listing, without a new filter: `.trash` is
    // hidden, which `is_listable` already excludes.
    let paths = listing(&vault);
    assert!(!paths.iter().any(|path| path == "projects/doomed.md"));
    assert!(!paths.iter().any(|path| path.starts_with(".trash")));
}

#[test]
fn a_trash_collision_gets_a_suffix_and_never_overwrites() {
    let (dir, vault) = vault();
    for round in ["first", "second", "third"] {
        create(&vault, "ideas/repeat.md", &format!("{round}\n"));
        delete_path(
            &vault,
            DeletePathRequest {
                path: "ideas/repeat.md".into(),
                expected_revision: None,
            },
        )
        .unwrap();
    }

    let trash = dir.path().join(".trash/ideas");
    assert_eq!(
        std::fs::read_to_string(trash.join("repeat.md")).unwrap(),
        "first\n"
    );
    // The suffix lands before the extension, not after it.
    assert_eq!(
        std::fs::read_to_string(trash.join("repeat_01.md")).unwrap(),
        "second\n"
    );
    assert_eq!(
        std::fs::read_to_string(trash.join("repeat_02.md")).unwrap(),
        "third\n"
    );
}

#[test]
fn deleting_a_folder_moves_the_whole_subtree_in_one_go() {
    let (dir, vault) = vault();
    create_folder(
        &vault,
        CreateFolderRequest {
            path: "projects/lens".into(),
        },
    )
    .unwrap();
    create(&vault, "projects/lens/a.md", "a\n");
    create(&vault, "projects/lens/b.md", "b\n");

    delete_path(
        &vault,
        DeletePathRequest {
            path: "projects/lens".into(),
            expected_revision: None,
        },
    )
    .unwrap();

    assert_eq!(
        std::fs::read_to_string(dir.path().join(".trash/projects/lens/b.md")).unwrap(),
        "b\n"
    );
    assert!(!listing(&vault).iter().any(|path| path.contains("lens")));
}

#[test]
fn a_stale_revision_refuses_the_delete() {
    let (_dir, vault) = vault();
    let first = create(&vault, "ideas/note.md", "one\n");
    write_document(
        &vault,
        WriteDocumentRequest {
            path: "ideas/note.md".into(),
            content: "two\n".into(),
            expected_revision: Some(Some(first.clone())),
        },
    )
    .unwrap();

    let error = delete_path(
        &vault,
        DeletePathRequest {
            path: "ideas/note.md".into(),
            expected_revision: Some(first),
        },
    )
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::RevisionConflict);
    assert_eq!(read_back(&vault, "ideas/note.md"), "two\n");
}

#[test]
fn deleting_a_note_leaves_other_notes_wikilinks_exactly_as_written() {
    let (_dir, vault) = vault();
    create(&vault, "ideas/target.md", "# Target\n");
    create(&vault, "ideas/source.md", "See [[target]].\n");

    move_path(
        &vault,
        MovePathRequest {
            from: "ideas/target.md".into(),
            to: "ideas/renamed.md".into(),
        },
    )
    .unwrap();

    // A rename is deliberately not a multi-file rewrite: nothing edits a note
    // the user did not open. The client warns using the link index and issues
    // ordinary revision-guarded writes if the user agrees.
    assert_eq!(read_back(&vault, "ideas/source.md"), "See [[target]].\n");
}

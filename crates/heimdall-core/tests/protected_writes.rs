//! Human edits to protected content (SPEC §6, §15, §17).
//!
//! The desktop shows `aios/` in its file tree, so a human can open their own
//! memories and entries — and must be able to fix them. What that cannot do is
//! break the invariants the protected tree exists to hold: an entry's
//! `created_at` and `type` stay the ones Heimdall wrote.

use camino::Utf8PathBuf;
use heimdall_core::commands::{
    create_entry, list_entries, read_entry, write_entry, CreateEntryRequest, EntryKind,
    ListEntriesRequest, ReadEntryRequest, WriteEntryRequest,
};
use heimdall_core::errors::ErrorCode;
use heimdall_core::paths::RelPath;
use heimdall_core::{template, Vault};

fn vault() -> (tempfile::TempDir, Vault) {
    let dir = tempfile::tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).unwrap();
    let vault = Vault::open(&root).unwrap();
    template::scaffold_full(&vault).unwrap();
    (dir, vault)
}

fn rel(raw: &str) -> RelPath {
    RelPath::parse(raw).unwrap()
}

/// Create one entry and return its id and current content.
fn seed_entry(vault: &Vault, kind: EntryKind) -> (String, String) {
    let created = create_entry(
        vault,
        CreateEntryRequest {
            kind,
            content: "# Summary\n\nAs written by the agent.\n".into(),
        },
    )
    .unwrap();
    let read = read_entry(
        vault,
        ReadEntryRequest {
            kind,
            id: created.id.clone(),
            start_line: None,
            max_lines: None,
        },
    )
    .unwrap();
    (created.id, read.content)
}

fn entry_text(vault: &Vault, kind: EntryKind, id: &str) -> String {
    read_entry(
        vault,
        ReadEntryRequest {
            kind,
            id: id.to_string(),
            start_line: None,
            max_lines: None,
        },
    )
    .unwrap()
    .content
}

fn entry_revision(vault: &Vault, kind: EntryKind, id: &str) -> heimdall_core::Revision {
    read_entry(
        vault,
        ReadEntryRequest {
            kind,
            id: id.to_string(),
            start_line: None,
            max_lines: None,
        },
    )
    .unwrap()
    .revision
}

#[test]
fn a_human_can_rewrite_an_entrys_body_while_heimdalls_fields_stay_put() {
    let (_dir, vault) = vault();
    let (id, original) = seed_entry(&vault, EntryKind::Conversation);
    let revision = entry_revision(&vault, EntryKind::Conversation, &id);

    let edited = original.replace("As written by the agent.", "Rewritten by hand.");
    write_entry(
        &vault,
        WriteEntryRequest {
            kind: EntryKind::Conversation,
            id: id.clone(),
            content: edited,
            expected_revision: revision,
        },
    )
    .unwrap();

    let stored = entry_text(&vault, EntryKind::Conversation, &id);
    assert!(stored.contains("Rewritten by hand."));
    assert!(stored.contains("type: conversation"));
    // The filename and created_at still name the same instant.
    let created_at = id.trim_end_matches(".md").replace('_', "T");
    assert!(stored.contains(&created_at[..10]), "{stored}");
}

#[test]
fn an_entry_edit_may_add_the_users_own_frontmatter_keys() {
    let (_dir, vault) = vault();
    let (id, original) = seed_entry(&vault, EntryKind::Notification);
    let revision = entry_revision(&vault, EntryKind::Notification, &id);

    // Exactly what a property editor produces: a `links:` list beside
    // the two fields Heimdall owns.
    let edited = original.replacen(
        "---\n\n",
        "links:\n  - \"[[profile]]\"\ntags: [inbox]\n---\n\n",
        1,
    );
    write_entry(
        &vault,
        WriteEntryRequest {
            kind: EntryKind::Notification,
            id: id.clone(),
            content: edited,
            expected_revision: revision,
        },
    )
    .unwrap();

    let stored = entry_text(&vault, EntryKind::Notification, &id);
    assert!(stored.contains("- \"[[profile]]\""));
    assert!(stored.contains("tags: [inbox]"));
    assert!(stored.contains("type: notification"));
}

#[test]
fn an_entry_edit_cannot_change_created_at() {
    let (_dir, vault) = vault();
    let (id, original) = seed_entry(&vault, EntryKind::Conversation);
    let revision = entry_revision(&vault, EntryKind::Conversation, &id);

    let tampered = original
        .lines()
        .map(|line| {
            if line.starts_with("created_at:") {
                "created_at: 1999-01-01T00:00:00Z".to_string()
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");

    let error = write_entry(
        &vault,
        WriteEntryRequest {
            kind: EntryKind::Conversation,
            id: id.clone(),
            content: tampered,
            expected_revision: revision,
        },
    )
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidInput);
    assert_eq!(error.details["parameter"], "created_at");
    assert!(!entry_text(&vault, EntryKind::Conversation, &id).contains("1999"));
}

#[test]
fn an_entry_edit_cannot_move_an_entry_between_kinds() {
    let (_dir, vault) = vault();
    let (id, original) = seed_entry(&vault, EntryKind::Conversation);
    let revision = entry_revision(&vault, EntryKind::Conversation, &id);

    let tampered = original.replace("type: conversation", "type: notification");
    let error = write_entry(
        &vault,
        WriteEntryRequest {
            kind: EntryKind::Conversation,
            id: id.clone(),
            content: tampered,
            expected_revision: revision,
        },
    )
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidInput);
    assert_eq!(error.details["parameter"], "type");
}

#[test]
fn an_entry_edit_that_drops_heimdalls_block_is_refused() {
    let (_dir, vault) = vault();
    let (id, _) = seed_entry(&vault, EntryKind::Conversation);
    let revision = entry_revision(&vault, EntryKind::Conversation, &id);

    let error = write_entry(
        &vault,
        WriteEntryRequest {
            kind: EntryKind::Conversation,
            id: id.clone(),
            content: "# Summary\n\nNo frontmatter at all.\n".into(),
            expected_revision: revision,
        },
    )
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidInput);
    assert_eq!(error.details["parameter"], "content");
    assert!(entry_text(&vault, EntryKind::Conversation, &id).contains("created_at:"));
}

#[test]
fn a_hand_made_entry_without_frontmatter_can_be_edited_freely() {
    let (_dir, vault) = vault();
    // Entries copied in by hand exist and must keep working; there is no
    // Heimdall-written frontmatter to preserve, so there is nothing to protect.
    let path = rel("aios/conversations/2026-08-16_10-00-00.md");
    vault.atomic_write(&path, b"Just a note.\n").unwrap();

    let listed = list_entries(
        &vault,
        ListEntriesRequest {
            kind: EntryKind::Conversation,
            limit: None,
        },
    )
    .unwrap();
    let id = listed.entries[0].id.clone();
    let revision = entry_revision(&vault, EntryKind::Conversation, &id);

    write_entry(
        &vault,
        WriteEntryRequest {
            kind: EntryKind::Conversation,
            id: id.clone(),
            content: "Edited freely.\n".into(),
            expected_revision: revision,
        },
    )
    .unwrap();
    assert_eq!(entry_text(&vault, EntryKind::Conversation, &id), "Edited freely.\n");
}

#[test]
fn entry_edits_are_revision_guarded() {
    let (_dir, vault) = vault();
    let (id, original) = seed_entry(&vault, EntryKind::Conversation);
    let stale = entry_revision(&vault, EntryKind::Conversation, &id);

    write_entry(
        &vault,
        WriteEntryRequest {
            kind: EntryKind::Conversation,
            id: id.clone(),
            content: original.replace("agent.", "agent, edited once."),
            expected_revision: stale.clone(),
        },
    )
    .unwrap();

    let error = write_entry(
        &vault,
        WriteEntryRequest {
            kind: EntryKind::Conversation,
            id: id.clone(),
            content: original.replace("agent.", "agent, edited twice."),
            expected_revision: stale,
        },
    )
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::RevisionConflict);
    assert!(entry_text(&vault, EntryKind::Conversation, &id).contains("edited once"));
}

#[test]
fn an_entry_whose_declared_type_disagrees_with_the_folder_is_refused_before_anything_else() {
    let (_dir, vault) = vault();
    let (id, original) = seed_entry(&vault, EntryKind::Conversation);
    let revision = entry_revision(&vault, EntryKind::Conversation, &id);

    // The content says "conversation" but the call targets the notifications
    // folder. Naming that disagreement beats reporting the file as missing,
    // which is what the caller would otherwise have to work out for itself.
    let error = write_entry(
        &vault,
        WriteEntryRequest {
            kind: EntryKind::Notification,
            id,
            content: original,
            expected_revision: revision,
        },
    )
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidInput);
    assert_eq!(error.details["parameter"], "type");
}

#[test]
fn editing_an_entry_that_does_not_exist_says_so() {
    let (_dir, vault) = vault();
    let (id, original) = seed_entry(&vault, EntryKind::Conversation);
    let revision = entry_revision(&vault, EntryKind::Conversation, &id);

    let error = write_entry(
        &vault,
        WriteEntryRequest {
            kind: EntryKind::Conversation,
            id: "2001-01-01_00-00-00.md".into(),
            content: original,
            expected_revision: revision,
        },
    )
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::NotFound);
    assert_eq!(error.details["id"], "2001-01-01_00-00-00.md");
}

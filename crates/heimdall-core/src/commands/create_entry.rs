//! `create_entry` — add a conversation summary or a notification (SPEC §10).
//!
//! Entries are create-only. Heimdall owns their frontmatter: caller content that
//! already begins with a frontmatter block is rejected rather than stripped,
//! merged, or silently rewritten (SPEC §6).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::commands::types::EntryKind;
use crate::errors::{Error, ErrorCode, Result};
use crate::limits;
use crate::revisions::Revision;
use crate::storage::Vault;
use crate::timestamps;

/// How many same-second collisions to absorb before giving up.
const MAX_COLLISION_SUFFIX: u32 = 99;

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateEntryRequest {
    pub kind: EntryKind,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct CreateEntryResponse {
    pub id: String,
    pub path: String,
    pub kind: EntryKind,
    pub created_at: String,
    pub size_bytes: usize,
    pub revision: Revision,
}

pub fn create_entry(vault: &Vault, request: CreateEntryRequest) -> Result<CreateEntryResponse> {
    vault.ensure_initialized()?;

    limits::check_write_size(
        request.content.len(),
        limits::ENTRY_CONTENT_MAX_BYTES,
        "entry",
    )?;
    reject_caller_frontmatter(&request.content)?;

    // The filename and the frontmatter timestamp come from one instant, so they
    // can never disagree (SPEC §6).
    let now = timestamps::now_utc();
    let created_at = timestamps::to_rfc3339(now);
    let stem = timestamps::entry_stem(now);
    let dir = request.kind.dir();

    let body = format!(
        "---\ncreated_at: {created_at}\ntype: {}\n---\n\n{}",
        request.kind, request.content
    );
    let bytes = body.as_bytes();

    for suffix in 0..=MAX_COLLISION_SUFFIX {
        let id = if suffix == 0 {
            format!("{stem}.md")
        } else {
            format!("{stem}_{suffix:02}.md")
        };
        let path = dir.join(&id);

        // O_EXCL, so a collision is detected by the filesystem rather than by a
        // check that another process could race.
        match vault.create_new(&path, bytes) {
            Ok(revision) => {
                return Ok(CreateEntryResponse {
                    id,
                    path: path.to_string(),
                    kind: request.kind,
                    created_at,
                    size_bytes: bytes.len(),
                    revision,
                })
            }
            Err(err) if err.code == ErrorCode::AlreadyExists => continue,
            Err(err) => return Err(err),
        }
    }

    Err(Error::already_exists(format!(
        "more than {MAX_COLLISION_SUFFIX} {} entries already exist for this second; retry",
        request.kind
    ))
    .with_detail("kind", request.kind.as_str()))
}

/// Reject content that opens with its own frontmatter block.
fn reject_caller_frontmatter(content: &str) -> Result<()> {
    let content = content.strip_prefix('\u{feff}').unwrap_or(content);
    let first_line = content.split('\n').next().unwrap_or_default();
    if first_line.trim_end_matches('\r') == "---" {
        return Err(Error::invalid_input(
            "entry content must not begin with a frontmatter block; Heimdall adds \
             created_at and type itself. Remove the leading \"---\" block and resend \
             the body only",
        )
        .with_detail("parameter", "content"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::list_entries::{list_entries, ListEntriesRequest};
    use crate::commands::read_entry::{read_entry, ReadEntryRequest};
    use crate::template;
    use camino::Utf8PathBuf;

    fn vault() -> (tempfile::TempDir, Vault) {
        let dir = tempfile::tempdir().unwrap();
        let root = Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).unwrap();
        let vault = Vault::open(&root).unwrap();
        template::scaffold_aios_only(&vault).unwrap();
        (dir, vault)
    }

    fn create(vault: &Vault, kind: EntryKind, content: &str) -> Result<CreateEntryResponse> {
        create_entry(
            vault,
            CreateEntryRequest {
                kind,
                content: content.to_string(),
            },
        )
    }

    #[test]
    fn an_entry_lands_in_the_folder_for_its_kind_with_owned_frontmatter() {
        let (_tmp, vault) = vault();
        let response = create(&vault, EntryKind::Notification, "# Notification\n").unwrap();

        assert_eq!(response.kind, EntryKind::Notification);
        assert!(response.path.starts_with("aios/notifications/"));

        let stored = read_entry(
            &vault,
            ReadEntryRequest {
                kind: EntryKind::Notification,
                id: response.id.clone(),
                start_line: None,
                max_lines: None,
            },
        )
        .unwrap();
        assert!(stored.content.starts_with("---\ncreated_at: "));
        assert!(stored.content.contains("type: notification\n"));
        assert!(stored.content.ends_with("# Notification\n"));
        assert_eq!(stored.revision, response.revision);
    }

    #[test]
    fn the_filename_and_frontmatter_agree_on_the_instant() {
        let (_tmp, vault) = vault();
        let response = create(&vault, EntryKind::Conversation, "# Summary\n").unwrap();

        let from_name = timestamps::parse_entry_stem(&response.id).unwrap();
        assert_eq!(timestamps::to_rfc3339(from_name), response.created_at);
        assert!(response.created_at.ends_with('Z'));
    }

    #[test]
    fn the_type_always_matches_the_folder() {
        let (_tmp, vault) = vault();
        for (kind, folder, label) in [
            (EntryKind::Conversation, "aios/conversations/", "type: conversation"),
            (EntryKind::Notification, "aios/notifications/", "type: notification"),
        ] {
            let response = create(&vault, kind, "body\n").unwrap();
            assert!(response.path.starts_with(folder));

            let stored = read_entry(
                &vault,
                ReadEntryRequest {
                    kind,
                    id: response.id,
                    start_line: None,
                    max_lines: None,
                },
            )
            .unwrap();
            assert!(stored.content.contains(label));
        }
    }

    #[test]
    fn caller_supplied_frontmatter_is_refused_rather_than_rewritten() {
        let (_tmp, vault) = vault();
        for content in [
            "---\ntype: sneaky\n---\n\nbody\n",
            "---\r\ntype: sneaky\r\n---\r\n",
            "\u{feff}---\ntype: sneaky\n---\n",
            "---",
        ] {
            let err = create(&vault, EntryKind::Conversation, content).unwrap_err();
            assert_eq!(err.code, ErrorCode::InvalidInput, "accepted {content:?}");
            assert!(err.message.contains("frontmatter"), "{}", err.message);
        }

        // Nothing was written by any of those attempts.
        let listed = list_entries(
            &vault,
            ListEntriesRequest {
                kind: EntryKind::Conversation,
                limit: None,
            },
        )
        .unwrap();
        assert!(listed.entries.is_empty());
    }

    #[test]
    fn a_horizontal_rule_further_down_is_ordinary_content() {
        let (_tmp, vault) = vault();
        let response = create(&vault, EntryKind::Conversation, "# Summary\n\n---\n\nMore.\n");
        assert!(response.is_ok());
    }

    #[test]
    fn same_second_collisions_get_numeric_suffixes_and_never_overwrite() {
        let (_tmp, vault) = vault();
        let mut ids = Vec::new();
        for index in 0..3 {
            let response = create(&vault, EntryKind::Conversation, &format!("body {index}\n")).unwrap();
            ids.push(response.id);
        }

        // All three coexist; each holds its own content.
        assert_eq!(ids.iter().collect::<std::collections::HashSet<_>>().len(), 3);
        for (index, id) in ids.iter().enumerate() {
            let stored = read_entry(
                &vault,
                ReadEntryRequest {
                    kind: EntryKind::Conversation,
                    id: id.clone(),
                    start_line: None,
                    max_lines: None,
                },
            )
            .unwrap();
            assert!(stored.content.ends_with(&format!("body {index}\n")), "{id}");
        }
    }

    #[test]
    fn a_created_entry_is_immediately_discoverable() {
        let (_tmp, vault) = vault();
        let response = create(&vault, EntryKind::Conversation, "# Summary\n").unwrap();

        let listed = list_entries(
            &vault,
            ListEntriesRequest {
                kind: EntryKind::Conversation,
                limit: None,
            },
        )
        .unwrap();
        assert_eq!(listed.entries[0].id, response.id);
        assert_eq!(listed.entries[0].created_at, response.created_at);
    }

    #[test]
    fn oversized_entry_content_is_rejected() {
        let (_tmp, vault) = vault();
        let too_big = "a".repeat(limits::ENTRY_CONTENT_MAX_BYTES + 1);
        let err = create(&vault, EntryKind::Conversation, &too_big).unwrap_err();
        assert_eq!(err.code, ErrorCode::LimitExceeded);
    }

    #[test]
    fn an_uninitialized_vault_refuses_to_create_anything() {
        let dir = tempfile::tempdir().unwrap();
        let root = Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).unwrap();
        let vault = Vault::open(&root).unwrap();

        let err = create(&vault, EntryKind::Conversation, "body\n").unwrap_err();
        assert_eq!(err.code, ErrorCode::NotInitialized);
    }
}

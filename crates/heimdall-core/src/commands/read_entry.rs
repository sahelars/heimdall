//! `read_entry` — bounded read of one selected entry.

use schemars::JsonSchema;
use serde::Deserialize;

use crate::commands::read_range::read_range;
use crate::commands::types::{EntryKind, ReadResult};
use crate::errors::{Error, Result};
use crate::limits;
use crate::paths::BaseName;
use crate::storage::Vault;

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReadEntryRequest {
    /// Selects the folder; must match where the entry actually lives.
    pub kind: EntryKind,
    /// A bare filename from `list_entries`, never a path.
    pub id: String,
    pub start_line: Option<u32>,
    pub max_lines: Option<u32>,
}

pub fn read_entry(vault: &Vault, request: ReadEntryRequest) -> Result<ReadResult> {
    vault.ensure_initialized()?;

    let start_line = limits::resolve_start_line(request.start_line)?;
    let max_lines = limits::resolve_max_lines(request.max_lines)?;
    let id = BaseName::parse_markdown(&request.id, "id")?;
    let path = request.kind.dir().join(id.as_str());

    if !vault.entry_exists(&path) {
        return Err(Error::not_found(format!(
            "no {} entry with id \"{id}\"",
            request.kind
        ))
        .with_detail("kind", request.kind.as_str())
        .with_detail("id", id.as_str()));
    }

    read_range(
        vault,
        &path,
        start_line,
        max_lines,
        limits::READ_TOTAL_BYTES_MAX,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::ErrorCode;
    use crate::template;
    use camino::Utf8PathBuf;

    fn vault() -> (tempfile::TempDir, Vault) {
        let dir = tempfile::tempdir().unwrap();
        let root = Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).unwrap();
        let vault = Vault::open(&root).unwrap();
        template::scaffold_aios_only(&vault).unwrap();
        (dir, vault)
    }

    fn write_entry(vault: &Vault, kind: EntryKind, name: &str, content: &[u8]) {
        vault.atomic_write(&kind.dir().join(name), content).unwrap();
    }

    fn read(vault: &Vault, kind: EntryKind, id: &str) -> Result<ReadResult> {
        read_entry(
            vault,
            ReadEntryRequest {
                kind,
                id: id.to_string(),
                start_line: None,
                max_lines: None,
            },
        )
    }

    #[test]
    fn reads_a_selected_entry_whole() {
        let (_tmp, vault) = vault();
        let body = b"---\ncreated_at: 2026-08-16T10:30:00Z\ntype: conversation\n---\n\n# Summary\n";
        write_entry(&vault, EntryKind::Conversation, "2026-08-16_10-30-00.md", body);

        let result = read(&vault, EntryKind::Conversation, "2026-08-16_10-30-00.md").unwrap();
        assert_eq!(result.path, "aios/conversations/2026-08-16_10-30-00.md");
        assert!(result.content.contains("type: conversation"));
        assert!(result.complete);
    }

    #[test]
    fn the_kind_must_match_where_the_entry_lives() {
        let (_tmp, vault) = vault();
        write_entry(&vault, EntryKind::Conversation, "2026-08-16_10-30-00.md", b"x");

        // Same id, wrong folder.
        let err = read(&vault, EntryKind::Notification, "2026-08-16_10-30-00.md").unwrap_err();
        assert_eq!(err.code, ErrorCode::NotFound);
        assert_eq!(err.details["kind"], "notification");
    }

    #[test]
    fn the_id_is_a_filename_and_never_a_path() {
        let (_tmp, vault) = vault();
        for id in [
            "../memories/memory.md",
            "../../agents.md",
            "sub/entry.md",
            "/etc/passwd.md",
            "entry.txt",
        ] {
            let err = read(&vault, EntryKind::Conversation, id).unwrap_err();
            assert_eq!(err.code, ErrorCode::InvalidInput, "accepted {id:?}");
        }
    }

    #[test]
    fn a_missing_entry_is_not_found() {
        let (_tmp, vault) = vault();
        let err = read(&vault, EntryKind::Conversation, "2026-01-01_00-00-00.md").unwrap_err();
        assert_eq!(err.code, ErrorCode::NotFound);
    }

    #[test]
    fn a_hand_made_entry_is_returned_unchanged() {
        let (_tmp, vault) = vault();
        write_entry(&vault, EntryKind::Notification, "2026_8_13.md", b"- this\n- is\n");

        let result = read(&vault, EntryKind::Notification, "2026_8_13.md").unwrap();
        assert_eq!(result.content, "- this\n- is\n");
    }

    #[test]
    fn long_entries_paginate_by_line_like_any_other_read() {
        let (_tmp, vault) = vault();
        write_entry(&vault, EntryKind::Conversation, "2026-08-16_10-30-00.md", b"1\n2\n3\n");

        let first = read_entry(
            &vault,
            ReadEntryRequest {
                kind: EntryKind::Conversation,
                id: "2026-08-16_10-30-00.md".to_string(),
                start_line: None,
                max_lines: Some(2),
            },
        )
        .unwrap();
        assert_eq!(first.content, "1\n2\n");
        assert_eq!(first.next_line, Some(3));
    }
}

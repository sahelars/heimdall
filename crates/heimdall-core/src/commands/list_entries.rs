//! `list_entries` — discover conversation summaries and notifications.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::commands::types::EntryKind;
use crate::errors::Result;
use crate::limits;
use crate::paths;
use crate::storage::Vault;
use crate::timestamps;

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListEntriesRequest {
    /// Required: entry folders are listed one kind at a time.
    pub kind: EntryKind,
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct EntryMeta {
    /// Bare filename. Pass it back as `id` to read the entry.
    pub id: String,
    pub kind: EntryKind,
    pub created_at: String,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ListEntriesResponse {
    pub entries: Vec<EntryMeta>,
    /// True when `limit` cut the list short. Raise `limit` to see the rest.
    pub truncated: bool,
}

pub fn list_entries(vault: &Vault, request: ListEntriesRequest) -> Result<ListEntriesResponse> {
    vault.ensure_initialized()?;
    let limit = limits::resolve_limit(request.limit)?;
    let dir = request.kind.dir();

    let mut entries: Vec<(OffsetDateTime, EntryMeta)> = Vec::new();
    for child in vault.children(&dir)? {
        if child.is_dir || !paths::is_listable(&child.name) {
            continue;
        }
        if !dir.join(&child.name).is_markdown() {
            continue;
        }

        // Heimdall names its own entries after the UTC instant that created
        // them, so the filename is authoritative and costs no file read. A file
        // a user added by hand in another editor still lists — it just falls
        // back to the filesystem's modification time.
        let created_at =
            timestamps::parse_entry_stem(&child.name).unwrap_or(child.meta.modified_at);

        entries.push((
            created_at,
            EntryMeta {
                id: child.name.clone(),
                kind: request.kind,
                created_at: timestamps::to_rfc3339(created_at),
                size_bytes: child.meta.size_bytes,
            },
        ));
    }

    // Newest first (SPEC §8); the filename breaks ties so collision variants of
    // one second keep a stable, deterministic order.
    entries.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.id.cmp(&a.1.id)));

    let truncated = entries.len() > limit;
    entries.truncate(limit);

    Ok(ListEntriesResponse {
        entries: entries.into_iter().map(|(_, meta)| meta).collect(),
        truncated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::RelPath;
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
        vault
            .atomic_write(&kind.dir().join(name), content)
            .unwrap();
    }

    fn list(vault: &Vault, kind: EntryKind) -> ListEntriesResponse {
        list_entries(vault, ListEntriesRequest { kind, limit: None }).unwrap()
    }

    fn ids(response: &ListEntriesResponse) -> Vec<&str> {
        response.entries.iter().map(|e| e.id.as_str()).collect()
    }

    #[test]
    fn entries_are_returned_newest_first() {
        let (_tmp, vault) = vault();
        write_entry(&vault, EntryKind::Conversation, "2026-08-14_10-00-00.md", b"a");
        write_entry(&vault, EntryKind::Conversation, "2026-08-16_10-00-00.md", b"b");
        write_entry(&vault, EntryKind::Conversation, "2026-08-15_10-00-00.md", b"c");

        assert_eq!(
            ids(&list(&vault, EntryKind::Conversation)),
            [
                "2026-08-16_10-00-00.md",
                "2026-08-15_10-00-00.md",
                "2026-08-14_10-00-00.md"
            ]
        );
    }

    #[test]
    fn each_kind_sees_only_its_own_folder() {
        let (_tmp, vault) = vault();
        write_entry(&vault, EntryKind::Conversation, "2026-08-16_10-00-00.md", b"a");
        write_entry(&vault, EntryKind::Notification, "2026-08-16_11-00-00.md", b"b");

        let conversations = list(&vault, EntryKind::Conversation);
        assert_eq!(ids(&conversations), ["2026-08-16_10-00-00.md"]);
        assert_eq!(conversations.entries[0].kind, EntryKind::Conversation);

        let notifications = list(&vault, EntryKind::Notification);
        assert_eq!(ids(&notifications), ["2026-08-16_11-00-00.md"]);
        assert_eq!(notifications.entries[0].kind, EntryKind::Notification);
    }

    #[test]
    fn created_at_comes_from_the_filename_when_heimdall_wrote_it() {
        let (_tmp, vault) = vault();
        write_entry(&vault, EntryKind::Conversation, "2026-08-16_10-30-00.md", b"a");

        let response = list(&vault, EntryKind::Conversation);
        assert_eq!(response.entries[0].created_at, "2026-08-16T10:30:00Z");
        assert_eq!(response.entries[0].size_bytes, 1);
    }

    #[test]
    fn a_hand_made_file_still_lists_using_its_modification_time() {
        let (_tmp, vault) = vault();
        // The shipped template carries exactly this shape of file.
        write_entry(&vault, EntryKind::Notification, "2026_8_13.md", b"- this\n");

        let response = list(&vault, EntryKind::Notification);
        assert_eq!(ids(&response), ["2026_8_13.md"]);
        let created_at = &response.entries[0].created_at;
        assert!(created_at.ends_with('Z'), "{created_at}");
        assert_ne!(created_at, "1970-01-01T00:00:00Z");
    }

    #[test]
    fn collision_variants_keep_a_deterministic_order() {
        let (_tmp, vault) = vault();
        for name in [
            "2026-08-16_10-30-00.md",
            "2026-08-16_10-30-00_01.md",
            "2026-08-16_10-30-00_02.md",
        ] {
            write_entry(&vault, EntryKind::Conversation, name, b"x");
        }

        assert_eq!(
            ids(&list(&vault, EntryKind::Conversation)),
            [
                "2026-08-16_10-30-00_02.md",
                "2026-08-16_10-30-00_01.md",
                "2026-08-16_10-30-00.md"
            ]
        );
    }

    #[test]
    fn a_small_limit_truncates_and_says_so() {
        let (_tmp, vault) = vault();
        for hour in 10..14 {
            write_entry(
                &vault,
                EntryKind::Conversation,
                &format!("2026-08-16_{hour}-00-00.md"),
                b"x",
            );
        }

        let response = list_entries(
            &vault,
            ListEntriesRequest {
                kind: EntryKind::Conversation,
                limit: Some(2),
            },
        )
        .unwrap();
        assert_eq!(response.entries.len(), 2);
        assert!(response.truncated);
        assert_eq!(response.entries[0].id, "2026-08-16_13-00-00.md");
    }

    #[test]
    fn an_empty_folder_lists_nothing_rather_than_failing() {
        let (_tmp, vault) = vault();
        let response = list(&vault, EntryKind::Conversation);
        assert!(response.entries.is_empty());
        assert!(!response.truncated);
    }

    #[test]
    fn sidecar_and_non_markdown_files_are_not_entries() {
        let (_tmp, vault) = vault();
        write_entry(&vault, EntryKind::Conversation, "2026-08-16_10-00-00.md", b"x");
        write_entry(&vault, EntryKind::Conversation, "notes.txt", b"x");
        write_entry(&vault, EntryKind::Conversation, ".DS_Store", b"x");
        vault
            .create_dir_all(&EntryKind::Conversation.dir().join("nested"))
            .unwrap();

        assert_eq!(ids(&list(&vault, EntryKind::Conversation)), ["2026-08-16_10-00-00.md"]);
    }

    #[test]
    fn listings_never_include_entry_content() {
        let (_tmp, vault) = vault();
        write_entry(
            &vault,
            EntryKind::Conversation,
            "2026-08-16_10-00-00.md",
            b"sensitive summary",
        );

        let response = list(&vault, EntryKind::Conversation);
        let json = serde_json::to_value(&response.entries[0]).unwrap();
        assert!(json.get("content").is_none());
        assert!(json.get("revision").is_none());
    }

    #[test]
    fn entry_folders_are_addressed_by_kind_never_by_path() {
        // The folder for each kind is fixed by the domain, not by caller input.
        assert_eq!(EntryKind::Conversation.dir().as_str(), paths::CONVERSATIONS_DIR);
        assert_eq!(EntryKind::Notification.dir().as_str(), paths::NOTIFICATIONS_DIR);
        assert!(RelPath::parse(paths::CONVERSATIONS_DIR).unwrap().is_in_aios());
    }
}

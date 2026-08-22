//! `list_memories` — discover the main memory and extended memory files.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::commands::types::MemoryKind;
use crate::errors::Result;
use crate::limits;
use crate::paths::{self, RelPath};
use crate::storage::Vault;
use crate::timestamps;

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListMemoriesRequest {
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct MemoryEntry {
    /// Bare filename. Pass an extended memory's name back as `extended`.
    pub name: String,
    pub kind: MemoryKind,
    pub path: String,
    pub size_bytes: u64,
    pub modified_at: String,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ListMemoriesResponse {
    pub memories: Vec<MemoryEntry>,
    /// True when `limit` cut the list short. Raise `limit` to see the rest.
    pub truncated: bool,
}

pub fn list_memories(vault: &Vault, request: ListMemoriesRequest) -> Result<ListMemoriesResponse> {
    vault.ensure_initialized()?;
    let limit = limits::resolve_limit(request.limit)?;

    // The main memory always leads: it is the entry point that links out to the
    // extended files (SPEC §8).
    let main_path = RelPath::parse(paths::MAIN_MEMORY_FILE)?;
    let main_meta = vault.meta(&main_path)?;
    let mut memories = vec![MemoryEntry {
        name: main_path.file_name().unwrap_or("memory.md").to_string(),
        kind: MemoryKind::Main,
        path: main_path.to_string(),
        size_bytes: main_meta.size_bytes,
        modified_at: timestamps::to_rfc3339(main_meta.modified_at),
    }];

    let extended_dir = RelPath::parse(paths::EXTENDED_DIR)?;
    for child in vault.children(&extended_dir)? {
        if child.is_dir || !paths::is_listable(&child.name) {
            continue;
        }
        let path = extended_dir.join(&child.name);
        if !path.is_markdown() {
            continue;
        }
        memories.push(MemoryEntry {
            name: child.name.clone(),
            kind: MemoryKind::Extended,
            path: path.to_string(),
            size_bytes: child.meta.size_bytes,
            modified_at: timestamps::to_rfc3339(child.meta.modified_at),
        });
    }

    // `children` already sorts by name, so extended memories follow the main
    // memory in filename order.
    let truncated = memories.len() > limit;
    memories.truncate(limit);

    Ok(ListMemoriesResponse {
        memories,
        truncated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::template;
    use camino::Utf8PathBuf;

    fn vault() -> (tempfile::TempDir, Vault) {
        let dir = tempfile::tempdir().unwrap();
        let root = Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).unwrap();
        let vault = Vault::open(&root).unwrap();
        template::scaffold_full(&vault).unwrap();
        (dir, vault)
    }

    fn write_extended(vault: &Vault, name: &str, content: &[u8]) {
        let path = RelPath::parse(paths::EXTENDED_DIR).unwrap().join(name);
        vault.atomic_write(&path, content).unwrap();
    }

    fn names(response: &ListMemoriesResponse) -> Vec<&str> {
        response.memories.iter().map(|m| m.name.as_str()).collect()
    }

    #[test]
    fn the_main_memory_leads_and_extended_memories_follow_by_filename() {
        let (_tmp, vault) = vault();
        write_extended(&vault, "zebra.md", b"z");
        write_extended(&vault, "alpha.md", b"a");

        let response = list_memories(&vault, ListMemoriesRequest::default()).unwrap();
        let names = names(&response);
        assert_eq!(names[0], "memory.md");
        assert_eq!(response.memories[0].kind, MemoryKind::Main);

        let alpha = names.iter().position(|n| *n == "alpha.md").unwrap();
        let zebra = names.iter().position(|n| *n == "zebra.md").unwrap();
        assert!(alpha < zebra);
        assert_eq!(response.memories[alpha].kind, MemoryKind::Extended);
    }

    #[test]
    fn listings_carry_metadata_but_no_content_or_revision() {
        let (_tmp, vault) = vault();
        write_extended(&vault, "topic.md", b"12345");

        let response = list_memories(&vault, ListMemoriesRequest::default()).unwrap();
        let topic = response
            .memories
            .iter()
            .find(|m| m.name == "topic.md")
            .unwrap();
        assert_eq!(topic.size_bytes, 5);
        assert_eq!(topic.path, "aios/memories/extended/topic.md");

        let json = serde_json::to_value(topic).unwrap();
        assert!(json.get("content").is_none());
        assert!(json.get("revision").is_none());
    }

    #[test]
    fn a_small_limit_truncates_and_says_so() {
        let (_tmp, vault) = vault();
        write_extended(&vault, "one.md", b"1");
        write_extended(&vault, "two.md", b"2");

        let response = list_memories(&vault, ListMemoriesRequest { limit: Some(1) }).unwrap();
        assert_eq!(names(&response), ["memory.md"]);
        assert!(response.truncated);
    }

    #[test]
    fn non_markdown_and_sidecar_files_are_not_memories() {
        let (_tmp, vault) = vault();
        write_extended(&vault, "notes.txt", b"x");
        write_extended(&vault, "real.md", b"x");
        let lock = RelPath::parse(paths::EXTENDED_DIR).unwrap().join(".real.md.lock");
        vault.atomic_write(&lock, b"").unwrap();

        let response = list_memories(&vault, ListMemoriesRequest::default()).unwrap();
        assert!(names(&response).contains(&"real.md"));
        assert!(!names(&response).contains(&"notes.txt"));
        assert!(!names(&response).iter().any(|n| n.contains("lock")));
    }
}

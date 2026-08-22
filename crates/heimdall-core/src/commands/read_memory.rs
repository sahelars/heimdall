//! `read_memory` — bounded read of the main memory or one extended memory.

use schemars::JsonSchema;
use serde::Deserialize;

use crate::commands::read_range::read_range;
use crate::commands::types::ReadResult;
use crate::errors::{Error, Result};
use crate::limits;
use crate::paths::{self, BaseName, RelPath};
use crate::storage::Vault;

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReadMemoryRequest {
    /// One extended memory filename, or `None` for the main memory.
    pub extended: Option<String>,
    pub start_line: Option<u32>,
    pub max_lines: Option<u32>,
}

pub fn read_memory(vault: &Vault, request: ReadMemoryRequest) -> Result<ReadResult> {
    vault.ensure_initialized()?;

    let start_line = limits::resolve_start_line(request.start_line)?;
    let max_lines = limits::resolve_max_lines(request.max_lines)?;
    let path = resolve_target(request.extended.as_deref())?;

    if !vault.entry_exists(&path) {
        return Err(Error::not_found(format!("memory \"{path}\" does not exist"))
            .with_detail("path", path.as_str()));
    }

    read_range(
        vault,
        &path,
        start_line,
        max_lines,
        limits::READ_TOTAL_BYTES_MAX,
    )
}

/// The file a memory request addresses.
///
/// `extended` takes one bare filename, never a path, so a caller cannot reach
/// outside `aios/memories/extended/` (SPEC §10).
pub(crate) fn resolve_target(extended: Option<&str>) -> Result<RelPath> {
    match extended {
        None => RelPath::parse(paths::MAIN_MEMORY_FILE),
        Some(name) => {
            let name = BaseName::parse_markdown(name, "extended")?;
            Ok(RelPath::parse(paths::EXTENDED_DIR)?.join(name.as_str()))
        }
    }
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
        template::scaffold_full(&vault).unwrap();
        (dir, vault)
    }

    #[test]
    fn reads_the_main_memory_by_default() {
        let (_tmp, vault) = vault();
        let result = read_memory(&vault, ReadMemoryRequest::default()).unwrap();

        assert_eq!(result.path, "aios/memories/memory.md");
        assert!(result.complete);
    }

    #[test]
    fn reads_one_named_extended_memory() {
        let (_tmp, vault) = vault();
        let path = RelPath::parse(paths::EXTENDED_DIR).unwrap().join("topic.md");
        vault.atomic_write(&path, b"one\ntwo\n").unwrap();

        let result = read_memory(
            &vault,
            ReadMemoryRequest {
                extended: Some("topic.md".to_string()),
                start_line: Some(2),
                max_lines: Some(1),
            },
        )
        .unwrap();

        assert_eq!(result.path, "aios/memories/extended/topic.md");
        assert_eq!(result.content, "two\n");
        assert!(result.complete);
    }

    #[test]
    fn extended_never_accepts_a_path() {
        let (_tmp, vault) = vault();
        for name in [
            "../memory.md",
            "sub/topic.md",
            "/etc/passwd.md",
            "..",
            "topic.txt",
        ] {
            let err = read_memory(
                &vault,
                ReadMemoryRequest {
                    extended: Some(name.to_string()),
                    ..Default::default()
                },
            )
            .unwrap_err();
            assert_eq!(err.code, ErrorCode::InvalidInput, "accepted {name:?}");
        }
    }

    #[test]
    fn a_missing_extended_memory_is_not_found() {
        let (_tmp, vault) = vault();
        let err = read_memory(
            &vault,
            ReadMemoryRequest {
                extended: Some("nope.md".to_string()),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert_eq!(err.code, ErrorCode::NotFound);
    }

    #[test]
    fn the_revision_matches_what_a_write_would_compare_against() {
        let (_tmp, vault) = vault();
        let path = RelPath::parse(paths::MAIN_MEMORY_FILE).unwrap();
        vault.atomic_write(&path, b"# Memory\n").unwrap();

        let result = read_memory(&vault, ReadMemoryRequest::default()).unwrap();
        assert_eq!(result.revision, crate::Revision::of_bytes(b"# Memory\n"));
    }
}

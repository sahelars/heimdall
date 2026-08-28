//! `write_memory` — replace the main memory or one extended memory (SPEC §10).
//!
//! This is a complete replacement guarded by optimistic concurrency, never a
//! patch, append, or automatic summarization. The revision a caller passes must
//! still match when the write happens, which is why the comparison and the
//! replacement both occur inside one cross-process lock.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::commands::read_memory::resolve_target;
use crate::commands::types::explicit_option;
use crate::errors::{Error, Result};
use crate::limits;
use crate::revisions::Revision;
use crate::storage::Vault;

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WriteMemoryRequest {
    pub content: String,
    /// One extended memory filename, or `None` to replace the main memory.
    #[serde(default)]
    pub extended: Option<String>,
    /// The revision from the latest read. Must be explicit `null` when creating
    /// a new extended memory; omitting the field entirely is an error.
    #[serde(default, deserialize_with = "explicit_option")]
    pub expected_revision: Option<Option<Revision>>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct WriteMemoryResponse {
    pub path: String,
    pub new_revision: Revision,
    pub size_bytes: usize,
    pub created: bool,
    /// Advisory guidance; the write already succeeded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

pub fn write_memory(vault: &Vault, request: WriteMemoryRequest) -> Result<WriteMemoryResponse> {
    vault.ensure_initialized()?;

    let is_main = request.extended.is_none();
    let path = resolve_target(request.extended.as_deref())?;
    let bytes = request.content.as_bytes();

    let (max_bytes, target_label) = if is_main {
        (limits::MAIN_MEMORY_MAX_BYTES, "main memory")
    } else {
        (limits::EXTENDED_MEMORY_MAX_BYTES, "extended memory")
    };
    limits::check_write_size(bytes.len(), max_bytes, target_label)?;

    let Some(expected_revision) = request.expected_revision else {
        return Err(Error::invalid_input(
            "expected_revision is required; pass the revision from the latest read, \
             or explicit null to create a new extended memory",
        )
        .with_detail("parameter", "expected_revision"));
    };

    let (new_revision, created) = vault.with_write_lock(&path, || {
        let exists = vault.is_file(&path);

        match (&expected_revision, exists) {
            (Some(expected), true) => {
                let current = Revision::of_bytes(&vault.read(&path)?);
                if *expected != current {
                    // The current revision is returned so the caller can re-read
                    // and merge. The current content is not: the caller asked to
                    // write, not to read.
                    return Err(Error::revision_conflict(format!(
                        "\"{path}\" changed since it was read; re-read it and retry"
                    ))
                    .with_detail("path", path.as_str())
                    .with_detail("current_revision", current.as_str()));
                }
            }
            (Some(_), false) => {
                return Err(Error::not_found(format!(
                    "\"{path}\" does not exist; pass expected_revision null to create it"
                ))
                .with_detail("path", path.as_str()));
            }
            (None, true) => {
                let current = Revision::of_bytes(&vault.read(&path)?);
                return Err(Error::revision_conflict(format!(
                    "\"{path}\" already exists; pass its revision instead of null to replace it"
                ))
                .with_detail("path", path.as_str())
                .with_detail("current_revision", current.as_str()));
            }
            (None, false) if is_main => {
                // ensure_initialized guarantees the main memory exists, so this
                // means it vanished between that check and the lock.
                return Err(Error::not_initialized(
                    "the main memory file is missing; run \"heimdall create\" against this vault",
                ));
            }
            (None, false) => {}
        }

        Ok((vault.atomic_write(&path, bytes)?, !exists))
    })?;

    Ok(WriteMemoryResponse {
        path: path.to_string(),
        new_revision,
        size_bytes: bytes.len(),
        created,
        warning: main_memory_warning(is_main, bytes.len()),
    })
}

/// Advise moving detail into `extended/` before the hard limit is reached.
///
/// The threshold is byte-based: token counts vary by model and tokenizer, so
/// Heimdall does not estimate them (SPEC §10).
fn main_memory_warning(is_main: bool, size: usize) -> Option<String> {
    (is_main && size >= limits::MAIN_MEMORY_WARN_BYTES).then(|| {
        format!(
            "the main memory is {size} bytes, within {} bytes of its {} byte limit; \
             move durable topic detail into descriptive files under \
             aios/memories/extended/ and link them from memory.md",
            limits::MAIN_MEMORY_MAX_BYTES - size,
            limits::MAIN_MEMORY_MAX_BYTES
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::read_memory::{read_memory, ReadMemoryRequest};
    use crate::errors::ErrorCode;
    use crate::paths::{self, RelPath};
    use crate::template;
    use camino::Utf8PathBuf;

    fn vault() -> (tempfile::TempDir, Vault) {
        let dir = tempfile::tempdir().unwrap();
        let root = Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).unwrap();
        let vault = Vault::open(&root).unwrap();
        template::scaffold_full(&vault).unwrap();
        (dir, vault)
    }

    fn main_revision(vault: &Vault) -> Revision {
        read_memory(vault, ReadMemoryRequest::default())
            .unwrap()
            .revision
    }

    fn write(
        vault: &Vault,
        content: &str,
        extended: Option<&str>,
        expected: Option<Option<Revision>>,
    ) -> Result<WriteMemoryResponse> {
        write_memory(
            vault,
            WriteMemoryRequest {
                content: content.to_string(),
                extended: extended.map(str::to_string),
                expected_revision: expected,
            },
        )
    }

    #[test]
    fn a_matching_revision_replaces_the_whole_file() {
        let (_tmp, vault) = vault();
        let revision = main_revision(&vault);

        let response = write(&vault, "# Memory\n\nNew.\n", None, Some(Some(revision))).unwrap();
        assert!(!response.created);
        assert_eq!(response.new_revision, Revision::of_bytes(b"# Memory\n\nNew.\n"));
        assert_eq!(
            read_memory(&vault, ReadMemoryRequest::default()).unwrap().content,
            "# Memory\n\nNew.\n"
        );
    }

    #[test]
    fn a_stale_revision_is_refused_and_reports_the_current_one() {
        let (_tmp, vault) = vault();
        let stale = main_revision(&vault);
        write(&vault, "first\n", None, Some(Some(stale.clone()))).unwrap();

        let err = write(&vault, "second\n", None, Some(Some(stale))).unwrap_err();
        assert_eq!(err.code, ErrorCode::RevisionConflict);
        assert_eq!(
            err.details["current_revision"],
            Revision::of_bytes(b"first\n").as_str()
        );
        // The conflict response must not hand back the file's content.
        assert!(!err.details.contains_key("content"));
        // And the write did not land.
        assert_eq!(
            read_memory(&vault, ReadMemoryRequest::default()).unwrap().content,
            "first\n"
        );
    }

    #[test]
    fn omitting_expected_revision_is_a_caller_error() {
        let (_tmp, vault) = vault();
        let err = write(&vault, "content\n", None, None).unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidInput);

        // Absent and explicit-null really do deserialize differently.
        let absent: WriteMemoryRequest =
            serde_json::from_str(r#"{"content":"x"}"#).unwrap();
        assert!(absent.expected_revision.is_none());
        let explicit: WriteMemoryRequest =
            serde_json::from_str(r#"{"content":"x","expected_revision":null}"#).unwrap();
        assert_eq!(explicit.expected_revision, Some(None));
    }

    #[test]
    fn a_null_revision_creates_a_new_extended_memory() {
        let (_tmp, vault) = vault();
        let response = write(&vault, "# Topic\n", Some("topic.md"), Some(None)).unwrap();

        assert!(response.created);
        assert_eq!(response.path, "aios/memories/extended/topic.md");
        assert_eq!(response.warning, None);
    }

    #[test]
    fn a_null_revision_against_an_existing_file_is_a_conflict_not_an_overwrite() {
        let (_tmp, vault) = vault();
        write(&vault, "original\n", Some("topic.md"), Some(None)).unwrap();

        let err = write(&vault, "clobbered\n", Some("topic.md"), Some(None)).unwrap_err();
        assert_eq!(err.code, ErrorCode::RevisionConflict);
        assert_eq!(
            read_memory(
                &vault,
                ReadMemoryRequest {
                    extended: Some("topic.md".to_string()),
                    ..Default::default()
                }
            )
            .unwrap()
            .content,
            "original\n"
        );
    }

    #[test]
    fn a_revision_for_a_file_that_does_not_exist_is_not_found() {
        let (_tmp, vault) = vault();
        let err = write(
            &vault,
            "x\n",
            Some("missing.md"),
            Some(Some(Revision::of_bytes(b"anything"))),
        )
        .unwrap_err();
        assert_eq!(err.code, ErrorCode::NotFound);
    }

    #[test]
    fn the_main_memory_stops_exactly_at_32_kib() {
        let (_tmp, vault) = vault();
        let revision = main_revision(&vault);

        let at_limit = "a".repeat(limits::MAIN_MEMORY_MAX_BYTES);
        let response = write(&vault, &at_limit, None, Some(Some(revision))).unwrap();
        assert_eq!(response.size_bytes, 32_768);

        let over = "a".repeat(limits::MAIN_MEMORY_MAX_BYTES + 1);
        let err = write(&vault, &over, None, Some(Some(response.new_revision))).unwrap_err();
        assert_eq!(err.code, ErrorCode::LimitExceeded);
    }

    #[test]
    fn a_large_main_memory_write_succeeds_with_advisory_guidance() {
        let (_tmp, vault) = vault();
        let revision = main_revision(&vault);

        let below = "a".repeat(limits::MAIN_MEMORY_WARN_BYTES - 1);
        let response = write(&vault, &below, None, Some(Some(revision))).unwrap();
        assert_eq!(response.warning, None);

        let at_warning = "a".repeat(limits::MAIN_MEMORY_WARN_BYTES);
        let response = write(&vault, &at_warning, None, Some(Some(response.new_revision))).unwrap();
        let warning = response.warning.expect("expected advisory warning");
        assert!(warning.contains("extended/"), "{warning}");
    }

    #[test]
    fn extended_memories_have_their_own_larger_limit() {
        let (_tmp, vault) = vault();
        let big = "a".repeat(limits::EXTENDED_MEMORY_MAX_BYTES);
        let response = write(&vault, &big, Some("topic.md"), Some(None)).unwrap();
        // No advisory warning: the guidance is about the main memory only.
        assert_eq!(response.warning, None);

        let too_big = "a".repeat(limits::EXTENDED_MEMORY_MAX_BYTES + 1);
        let err = write(
            &vault,
            &too_big,
            Some("other.md"),
            Some(None),
        )
        .unwrap_err();
        assert_eq!(err.code, ErrorCode::LimitExceeded);
    }

    #[test]
    fn a_rejected_write_leaves_no_temp_files_behind() {
        let (_tmp, vault) = vault();
        write(&vault, "x\n", Some("topic.md"), Some(None)).unwrap();
        let _ = write(&vault, "y\n", Some("topic.md"), Some(None));

        let leftovers: Vec<_> = vault
            .children(&RelPath::parse(paths::EXTENDED_DIR).unwrap())
            .unwrap()
            .into_iter()
            .map(|c| c.name)
            .filter(|name| name.contains(".tmp-"))
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
        // The rejected second write left the original content in place.
        assert_eq!(
            vault
                .read(&RelPath::parse(paths::EXTENDED_DIR).unwrap().join("topic.md"))
                .unwrap(),
            b"x\n"
        );
    }

    #[test]
    fn writes_cannot_escape_the_extended_directory() {
        let (_tmp, vault) = vault();
        for name in ["../memory.md", "../../note.md", "sub/topic.md"] {
            let err = write(&vault, "x\n", Some(name), Some(None)).unwrap_err();
            assert_eq!(err.code, ErrorCode::InvalidInput, "accepted {name:?}");
        }
        // The main memory is untouched by those attempts.
        assert!(vault.is_file(&RelPath::parse(paths::MAIN_MEMORY_FILE).unwrap()));
    }
}

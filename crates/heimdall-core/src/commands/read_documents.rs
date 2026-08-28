//! `read_documents` — bounded reads of selected ordinary notes (SPEC §10).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::commands::read_range::read_range;
use crate::commands::types::ReadResult;
use crate::errors::{Error, ErrorCode, Result};
use crate::limits;
use crate::paths::RelPath;
use crate::storage::Vault;

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DocumentSelection {
    pub path: String,
    pub start_line: Option<u32>,
    pub max_lines: Option<u32>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReadDocumentsRequest {
    pub documents: Vec<DocumentSelection>,
    pub max_total_bytes: Option<u32>,
}

/// Why a requested selection came back without content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SkipReason {
    /// Earlier selections consumed the request's byte budget. Request this
    /// document again on its own to read it.
    ByteBudgetExhausted,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct DocumentRead {
    pub path: String,
    pub returned: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(flatten)]
    pub result: Option<ReadResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<SkipReason>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ReadDocumentsResponse {
    pub documents: Vec<DocumentRead>,
    pub total_bytes: usize,
    /// True when at least one selection was not returned.
    pub truncated: bool,
}

pub fn read_documents(
    vault: &Vault,
    request: ReadDocumentsRequest,
) -> Result<ReadDocumentsResponse> {
    vault.ensure_initialized()?;

    if request.documents.is_empty() {
        return Err(Error::invalid_input(
            "select at least one document; use list_documents to discover paths",
        )
        .with_detail("parameter", "documents"));
    }
    if request.documents.len() > limits::READ_MAX_DOCUMENTS {
        return Err(Error::limit_exceeded(format!(
            "{} documents requested, above the maximum of {} per call",
            request.documents.len(),
            limits::READ_MAX_DOCUMENTS
        ))
        .with_detail("requested", request.documents.len())
        .with_detail("maximum", limits::READ_MAX_DOCUMENTS));
    }

    let budget = limits::resolve_max_total_bytes(request.max_total_bytes)?;

    // Validate every selection before reading any of it, so a typo in the last
    // path does not leave the caller holding a partial result set.
    let mut selections = Vec::with_capacity(request.documents.len());
    for selection in &request.documents {
        let path = RelPath::parse_markdown(&selection.path)?;
        path.deny_aios()?;
        // A name that is not there at all is NOT_FOUND. A name that is there
        // but resolves outside the vault falls through to the read, which
        // reports PATH_OUTSIDE_VAULT — the caller learns which problem it has.
        if !vault.entry_exists(&path) {
            return Err(Error::not_found(format!("document \"{path}\" does not exist"))
                .with_detail("path", path.as_str()));
        }
        if vault.is_dir(&path) {
            return Err(Error::invalid_input(format!(
                "\"{path}\" is a directory; use list_documents to find files inside it"
            ))
            .with_detail("path", path.as_str()));
        }
        selections.push((
            path,
            limits::resolve_start_line(selection.start_line)?,
            limits::resolve_max_lines(selection.max_lines)?,
        ));
    }

    let mut documents = Vec::with_capacity(selections.len());
    let mut used = 0usize;

    for (path, start_line, max_lines) in selections {
        let remaining = budget - used;
        match read_range(vault, &path, start_line, max_lines, remaining) {
            Ok(result) => {
                used += result.content.len();
                documents.push(DocumentRead {
                    path: path.to_string(),
                    returned: true,
                    result: Some(result),
                    reason: None,
                });
            }
            // A line too large for the remaining budget is only a per-selection
            // outcome once earlier documents have spent some of it. With the
            // full budget still available the request can never be satisfied,
            // so the caller needs the error rather than a silent skip.
            Err(err) if err.code == ErrorCode::LimitExceeded && used > 0 => {
                documents.push(DocumentRead {
                    path: path.to_string(),
                    returned: false,
                    result: None,
                    reason: Some(SkipReason::ByteBudgetExhausted),
                });
            }
            Err(err) => return Err(err),
        }
    }

    let truncated = documents.iter().any(|doc| !doc.returned);
    Ok(ReadDocumentsResponse {
        documents,
        total_bytes: used,
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
        template::scaffold_aios_only(&vault).unwrap();
        (dir, vault)
    }

    fn write(vault: &Vault, path: &str, content: &[u8]) {
        let path = RelPath::parse(path).unwrap();
        vault.create_dir_all(&path.parent()).unwrap();
        vault.atomic_write(&path, content).unwrap();
    }

    fn select(path: &str) -> DocumentSelection {
        DocumentSelection {
            path: path.to_string(),
            start_line: None,
            max_lines: None,
        }
    }

    #[test]
    fn reads_several_documents_in_the_requested_order() {
        let (_tmp, vault) = vault();
        write(&vault, "a.md", b"alpha\n");
        write(&vault, "projects/b.md", b"beta\n");

        let response = read_documents(
            &vault,
            ReadDocumentsRequest {
                documents: vec![select("projects/b.md"), select("a.md")],
                max_total_bytes: None,
            },
        )
        .unwrap();

        assert_eq!(response.documents.len(), 2);
        assert_eq!(response.documents[0].path, "projects/b.md");
        assert_eq!(
            response.documents[1].result.as_ref().unwrap().content,
            "alpha\n"
        );
        assert_eq!(response.total_bytes, 11);
        assert!(!response.truncated);
    }

    #[test]
    fn the_byte_budget_marks_later_selections_as_not_returned() {
        let (_tmp, vault) = vault();
        write(&vault, "a.md", b"aaaaaaaa\n");
        write(&vault, "b.md", b"bbbbbbbb\n");

        let response = read_documents(
            &vault,
            ReadDocumentsRequest {
                documents: vec![select("a.md"), select("b.md")],
                max_total_bytes: Some(9),
            },
        )
        .unwrap();

        assert!(response.documents[0].returned);
        assert!(!response.documents[1].returned);
        assert_eq!(
            response.documents[1].reason,
            Some(SkipReason::ByteBudgetExhausted)
        );
        assert!(response.truncated);
    }

    #[test]
    fn a_document_that_can_never_fit_the_budget_is_an_error_not_a_skip() {
        let (_tmp, vault) = vault();
        write(&vault, "a.md", b"aaaaaaaaaaaaaaaaaaaa\n");

        let err = read_documents(
            &vault,
            ReadDocumentsRequest {
                documents: vec![select("a.md")],
                max_total_bytes: Some(8),
            },
        )
        .unwrap_err();
        assert_eq!(err.code, ErrorCode::LimitExceeded);
    }

    #[test]
    fn selections_are_validated_before_anything_is_read() {
        let (_tmp, vault) = vault();
        write(&vault, "a.md", b"alpha\n");

        let err = read_documents(
            &vault,
            ReadDocumentsRequest {
                documents: vec![select("a.md"), select("missing.md")],
                max_total_bytes: None,
            },
        )
        .unwrap_err();
        assert_eq!(err.code, ErrorCode::NotFound);
    }

    #[test]
    fn protected_content_cannot_be_read_as_a_document() {
        let (_tmp, vault) = vault();
        for path in [
            "aios/memories/memory.md",
            "AIOS/memories/memory.md",
            "aios/attachments/notes.md",
        ] {
            let err = read_documents(
                &vault,
                ReadDocumentsRequest {
                    documents: vec![select(path)],
                    max_total_bytes: None,
                },
            )
            .unwrap_err();
            assert_eq!(err.code, ErrorCode::NotFound, "{path}");
        }
    }

    #[test]
    fn traversal_out_of_the_vault_is_refused() {
        let (_tmp, vault) = vault();
        for path in ["../escape.md", "projects/../../escape.md", "/etc/hosts.md"] {
            let err = read_documents(
                &vault,
                ReadDocumentsRequest {
                    documents: vec![select(path)],
                    max_total_bytes: None,
                },
            )
            .unwrap_err();
            assert!(
                matches!(err.code, ErrorCode::InvalidInput | ErrorCode::NotFound),
                "{path} produced {:?}",
                err.code
            );
        }
    }

    #[test]
    fn the_file_count_cap_is_enforced() {
        let (_tmp, vault) = vault();
        for index in 0..11 {
            write(&vault, &format!("n{index}.md"), b"x\n");
        }

        let err = read_documents(
            &vault,
            ReadDocumentsRequest {
                documents: (0..11).map(|i| select(&format!("n{i}.md"))).collect(),
                max_total_bytes: None,
            },
        )
        .unwrap_err();
        assert_eq!(err.code, ErrorCode::LimitExceeded);
        assert_eq!(err.details["maximum"], 10);

        assert_eq!(
            read_documents(
                &vault,
                ReadDocumentsRequest {
                    documents: vec![],
                    max_total_bytes: None,
                },
            )
            .unwrap_err()
            .code,
            ErrorCode::InvalidInput
        );
    }

    #[test]
    fn per_file_line_ranges_are_independent() {
        let (_tmp, vault) = vault();
        write(&vault, "a.md", b"1\n2\n3\n4\n");
        write(&vault, "b.md", b"x\ny\n");

        let response = read_documents(
            &vault,
            ReadDocumentsRequest {
                documents: vec![
                    DocumentSelection {
                        path: "a.md".to_string(),
                        start_line: Some(3),
                        max_lines: Some(1),
                    },
                    select("b.md"),
                ],
                max_total_bytes: None,
            },
        )
        .unwrap();

        let first = response.documents[0].result.as_ref().unwrap();
        assert_eq!(first.content, "3\n");
        assert_eq!(first.next_line, Some(4));
        assert!(response.documents[1].result.as_ref().unwrap().complete);
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_out_of_the_vault_is_named_as_an_escape_not_a_missing_file() {
        let (tmp, vault) = vault();
        let outside = tmp.path().parent().unwrap().join("heimdall-read-docs-secret.md");
        std::fs::write(&outside, b"secret\n").unwrap();
        std::os::unix::fs::symlink(&outside, tmp.path().join("escape.md")).unwrap();

        let err = read_documents(
            &vault,
            ReadDocumentsRequest {
                documents: vec![select("escape.md")],
                max_total_bytes: None,
            },
        )
        .unwrap_err();
        assert_eq!(err.code, ErrorCode::PathOutsideVault);
        assert!(!err.message.contains("secret"), "{}", err.message);

        // A name that simply is not there stays NOT_FOUND.
        let missing = read_documents(
            &vault,
            ReadDocumentsRequest {
                documents: vec![select("absent.md")],
                max_total_bytes: None,
            },
        )
        .unwrap_err();
        assert_eq!(missing.code, ErrorCode::NotFound);

        std::fs::remove_file(outside).unwrap();
    }

    #[test]
    fn non_markdown_selections_are_rejected() {
        let (_tmp, vault) = vault();
        write(&vault, "image.png", b"x");

        assert_eq!(
            read_documents(
                &vault,
                ReadDocumentsRequest {
                    documents: vec![select("image.png")],
                    max_total_bytes: None,
                },
            )
            .unwrap_err()
            .code,
            ErrorCode::InvalidInput
        );
    }
}

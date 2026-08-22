//! `list_documents` — bounded discovery of ordinary notes (SPEC §8, §10).
//!
//! Results are metadata only and never include content or a revision: hashing
//! during a listing would read every file. The protected `aios/` tree is
//! excluded at every depth.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::commands::types::DocumentKind;
use crate::errors::{Error, Result};
use crate::limits;
use crate::paths::{self, RelPath};
use crate::storage::Vault;
use crate::timestamps;

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListDocumentsRequest {
    /// Directory to list, relative to the vault root. Defaults to the root.
    pub path: Option<String>,
    #[serde(default)]
    pub recursive: bool,
    pub max_depth: Option<u32>,
    /// The `next_cursor` of a previous page: listing resumes strictly after it.
    pub cursor: Option<String>,
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DocumentEntry {
    pub path: String,
    pub kind: DocumentKind,
    /// Present for documents; a directory has no meaningful content size.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
    pub modified_at: String,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ListDocumentsResponse {
    pub entries: Vec<DocumentEntry>,
    /// Pass back as `cursor` to continue. `None` means the listing is finished.
    pub next_cursor: Option<String>,
    /// True when the per-call filesystem scan limit stopped the walk early.
    pub scan_guard_hit: bool,
}

pub fn list_documents(
    vault: &Vault,
    request: ListDocumentsRequest,
) -> Result<ListDocumentsResponse> {
    vault.ensure_initialized()?;

    let limit = limits::resolve_limit(request.limit)?;
    let max_level = if request.recursive {
        limits::resolve_max_depth(request.max_depth)?
    } else {
        if request.max_depth.is_some() {
            return Err(Error::invalid_input(
                "max_depth applies only to a recursive listing; set recursive to true",
            )
            .with_detail("parameter", "max_depth"));
        }
        1
    };

    let root = match &request.path {
        Some(raw) => RelPath::parse(raw)?,
        None => RelPath::root(),
    };
    root.deny_aios()?;
    if !vault.is_dir(&root) {
        return Err(if vault.exists(&root) {
            Error::invalid_input(format!("\"{root}\" is a document, not a directory"))
                .with_detail("path", root.as_str())
        } else {
            Error::not_found(format!("directory \"{root}\" does not exist"))
                .with_detail("path", root.as_str())
        });
    }

    let cursor = match &request.cursor {
        // A cursor is just the last path returned, so it validates as one.
        Some(raw) => Some(
            RelPath::parse(raw)
                .map_err(|_| {
                    Error::invalid_input("cursor is malformed; pass back the next_cursor value verbatim")
                        .with_detail("parameter", "cursor")
                })?
                .to_string(),
        ),
        None => None,
    };

    let mut walk = Walk {
        vault,
        cursor: cursor.as_deref(),
        max_level,
        examined: 0,
        guard_hit: false,
        found: Vec::new(),
    };
    walk.visit(&root, 1)?;

    let Walk {
        guard_hit,
        mut found,
        ..
    } = walk;

    // Sorting by relative path gives one deterministic order for the whole
    // listing, independent of directory traversal order, which is what makes
    // "resume strictly after this path" well defined.
    found.sort_by(|a, b| a.path.cmp(&b.path));

    let more_remain = found.len() > limit;
    found.truncate(limit);

    let next_cursor = match found.last() {
        Some(last) if more_remain || guard_hit => Some(last.path.clone()),
        _ => None,
    };

    Ok(ListDocumentsResponse {
        entries: found,
        next_cursor,
        scan_guard_hit: guard_hit,
    })
}

struct Walk<'a> {
    vault: &'a Vault,
    cursor: Option<&'a str>,
    max_level: usize,
    examined: usize,
    guard_hit: bool,
    found: Vec<DocumentEntry>,
}

impl Walk<'_> {
    fn visit(&mut self, dir: &RelPath, level: usize) -> Result<()> {
        if self.guard_hit {
            return Ok(());
        }
        for name in self.vault.child_names(dir)? {
            if !paths::is_listable(&name) {
                continue;
            }

            let path = dir.join(&name);
            // `aios/` is protected at every depth, not just at the root.
            if path.is_in_aios() {
                continue;
            }

            // Names alone answer most of the question, so entries this call has
            // no use for cost nothing: no metadata, no scan budget.
            let past_cursor = self.cursor.is_none_or(|cursor| path.as_str() > cursor);
            let is_markdown = path.is_markdown();
            let may_descend = level < self.max_level && self.descends_into(&path);
            if !past_cursor && !may_descend && is_markdown {
                continue;
            }

            let Some(child) = self.vault.resolve_child(dir, &name) else {
                // Unresolvable inside the vault — an escaping symlink, or an
                // entry deleted mid-walk. Neither is listable content.
                continue;
            };

            if child.is_dir {
                if !may_descend && !past_cursor {
                    continue;
                }
                if past_cursor && !self.charge() {
                    return Ok(());
                }
                if past_cursor {
                    self.push(&path, DocumentKind::Directory, None, &child.meta);
                }
                if may_descend {
                    self.visit(&path, level + 1)?;
                    if self.guard_hit {
                        return Ok(());
                    }
                }
            } else if is_markdown && past_cursor {
                // Vaults may hold arbitrary files, but V1 exposes no operation
                // that can read one, so listing them would be a dead end.
                if !self.charge() {
                    return Ok(());
                }
                self.push(
                    &path,
                    DocumentKind::Document,
                    Some(child.meta.size_bytes),
                    &child.meta,
                );
            }
        }
        Ok(())
    }

    /// Spend one unit of the scan budget, reporting whether the walk may go on.
    ///
    /// Only entries beyond the cursor are charged. Ground a previous page
    /// already covered was paid for by that page, and charging for it again
    /// would make the far end of a large vault permanently unreachable.
    fn charge(&mut self) -> bool {
        self.examined += 1;
        if self.examined > limits::LIST_SCAN_GUARD {
            self.guard_hit = true;
            return false;
        }
        true
    }

    fn push(
        &mut self,
        path: &RelPath,
        kind: DocumentKind,
        size_bytes: Option<u64>,
        meta: &crate::storage::FileMeta,
    ) {
        self.found.push(DocumentEntry {
            path: path.to_string(),
            kind,
            size_bytes,
            modified_at: timestamps::to_rfc3339(meta.modified_at),
        });
    }

    /// Skip a subtree that lies entirely behind the cursor.
    ///
    /// Every path under `dir` starts with `dir/`, and `/` is `0x2F`, so all of
    /// them sort below `dir` followed by `0x30`. If the cursor is already at or
    /// past that bound the subtree holds nothing new, and not descending keeps
    /// later pages from re-scanning ground already covered.
    fn descends_into(&self, dir: &RelPath) -> bool {
        match self.cursor {
            None => true,
            Some(cursor) => cursor < format!("{dir}0").as_str(),
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
        template::scaffold_aios_only(&vault).unwrap();
        (dir, vault)
    }

    fn write(vault: &Vault, path: &str, content: &[u8]) {
        let path = RelPath::parse(path).unwrap();
        vault.create_dir_all(&path.parent()).unwrap();
        vault.atomic_write(&path, content).unwrap();
    }

    fn list(vault: &Vault, request: ListDocumentsRequest) -> ListDocumentsResponse {
        list_documents(vault, request).unwrap()
    }

    fn paths_of(response: &ListDocumentsResponse) -> Vec<&str> {
        response.entries.iter().map(|e| e.path.as_str()).collect()
    }

    #[test]
    fn a_non_recursive_listing_returns_only_immediate_children() {
        let (_tmp, vault) = vault();
        write(&vault, "top.md", b"x");
        write(&vault, "projects/deep.md", b"x");

        let response = list(&vault, ListDocumentsRequest::default());
        assert_eq!(paths_of(&response), ["projects", "top.md"]);
        assert_eq!(response.entries[0].kind, DocumentKind::Directory);
        assert_eq!(response.entries[1].size_bytes, Some(1));
    }

    #[test]
    fn a_recursive_listing_walks_to_the_requested_depth() {
        let (_tmp, vault) = vault();
        write(&vault, "a/b/c/d/deep.md", b"x");

        let deep = list(
            &vault,
            ListDocumentsRequest {
                recursive: true,
                max_depth: Some(5),
                ..Default::default()
            },
        );
        assert!(paths_of(&deep).contains(&"a/b/c/d/deep.md"));

        let shallow = list(
            &vault,
            ListDocumentsRequest {
                recursive: true,
                max_depth: Some(2),
                ..Default::default()
            },
        );
        assert_eq!(paths_of(&shallow), ["a", "a/b"]);
    }

    #[test]
    fn protected_content_is_invisible_at_every_depth() {
        let (_tmp, vault) = vault();
        write(&vault, "notes.md", b"x");

        let response = list(
            &vault,
            ListDocumentsRequest {
                recursive: true,
                max_depth: Some(16),
                ..Default::default()
            },
        );
        assert!(
            !paths_of(&response).iter().any(|p| p.starts_with("aios")),
            "{:?}",
            paths_of(&response)
        );

        // Asking for it directly is a miss, in either casing.
        for raw in ["aios", "AIOS", "aios/memories"] {
            let err = list_documents(
                &vault,
                ListDocumentsRequest {
                    path: Some(raw.to_string()),
                    ..Default::default()
                },
            )
            .unwrap_err();
            assert_eq!(err.code, ErrorCode::NotFound, "{raw}");
        }
    }

    #[test]
    fn only_directories_and_markdown_are_listed() {
        let (_tmp, vault) = vault();
        write(&vault, "note.md", b"x");
        write(&vault, "image.png", b"x");
        write(&vault, "archive.tar.gz", b"x");
        write(&vault, ".hidden.md", b"x");
        write(&vault, ".DS_Store", b"x");

        let response = list(&vault, ListDocumentsRequest::default());
        assert_eq!(paths_of(&response), ["note.md"]);
    }

    #[test]
    fn pagination_walks_the_whole_listing_exactly_once() {
        let (_tmp, vault) = vault();
        for index in 0..10 {
            write(&vault, &format!("note_{index:02}.md"), b"x");
        }

        let mut seen = Vec::new();
        let mut cursor = None;
        loop {
            let response = list(
                &vault,
                ListDocumentsRequest {
                    cursor: cursor.clone(),
                    limit: Some(3),
                    ..Default::default()
                },
            );
            assert!(response.entries.len() <= 3);
            seen.extend(response.entries.iter().map(|e| e.path.clone()));
            cursor = response.next_cursor;
            if cursor.is_none() {
                break;
            }
        }

        let expected: Vec<String> = (0..10).map(|i| format!("note_{i:02}.md")).collect();
        assert_eq!(seen, expected);
    }

    #[test]
    fn pagination_is_stable_across_nested_directories() {
        let (_tmp, vault) = vault();
        write(&vault, "a/one.md", b"x");
        write(&vault, "a/two.md", b"x");
        write(&vault, "b/three.md", b"x");
        write(&vault, "top.md", b"x");

        let all = list(
            &vault,
            ListDocumentsRequest {
                recursive: true,
                ..Default::default()
            },
        );
        let expected = paths_of(&all)
            .iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>();

        let mut seen = Vec::new();
        let mut cursor = None;
        loop {
            let page = list(
                &vault,
                ListDocumentsRequest {
                    recursive: true,
                    cursor: cursor.clone(),
                    limit: Some(2),
                    ..Default::default()
                },
            );
            seen.extend(page.entries.iter().map(|e| e.path.clone()));
            cursor = page.next_cursor;
            if cursor.is_none() {
                break;
            }
        }
        assert_eq!(seen, expected);
    }

    #[test]
    fn the_last_page_reports_no_cursor() {
        let (_tmp, vault) = vault();
        write(&vault, "only.md", b"x");

        let response = list(&vault, ListDocumentsRequest::default());
        assert_eq!(response.next_cursor, None);
        assert!(!response.scan_guard_hit);
    }

    #[test]
    fn ordering_is_by_relative_path_not_traversal_order() {
        let (_tmp, vault) = vault();
        write(&vault, "a/inner.md", b"x");
        write(&vault, "a.md", b"x");
        write(&vault, "b.md", b"x");

        let response = list(
            &vault,
            ListDocumentsRequest {
                recursive: true,
                ..Default::default()
            },
        );
        assert_eq!(paths_of(&response), ["a", "a.md", "a/inner.md", "b.md"]);
    }

    #[test]
    fn a_malformed_cursor_is_rejected() {
        let (_tmp, vault) = vault();
        let err = list_documents(
            &vault,
            ListDocumentsRequest {
                cursor: Some("../escape".to_string()),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidInput);
    }

    #[test]
    fn listing_a_missing_or_non_directory_path_is_reported_precisely() {
        let (_tmp, vault) = vault();
        write(&vault, "note.md", b"x");

        let missing = list_documents(
            &vault,
            ListDocumentsRequest {
                path: Some("nowhere".to_string()),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert_eq!(missing.code, ErrorCode::NotFound);

        let a_file = list_documents(
            &vault,
            ListDocumentsRequest {
                path: Some("note.md".to_string()),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert_eq!(a_file.code, ErrorCode::InvalidInput);
    }

    #[test]
    fn max_depth_without_recursive_is_a_caller_mistake() {
        let (_tmp, vault) = vault();
        let err = list_documents(
            &vault,
            ListDocumentsRequest {
                max_depth: Some(3),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidInput);
    }

    #[test]
    fn listings_carry_metadata_but_never_content_or_revisions() {
        let (_tmp, vault) = vault();
        write(&vault, "note.md", b"secret content");

        let response = list(&vault, ListDocumentsRequest::default());
        let json = serde_json::to_value(&response.entries[0]).unwrap();
        assert!(json.get("content").is_none());
        assert!(json.get("revision").is_none());
        assert!(json["modified_at"].as_str().unwrap().ends_with('Z'));
    }
}

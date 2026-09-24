//! `read` — the one way to look at a vault (SPEC §10).
//!
//! A folder comes back as a bounded listing and a note as a bounded range of
//! its lines, so one verb covers both "what is here" and "what does it say".
//! Omitting the path reads the vault root. Every result says whether the thing
//! read is locked, so a caller learns before it writes that it cannot.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::commands::listing::{self, Listing, ListingOptions};
use crate::commands::read_range::read_range;
use crate::commands::types::{DocumentKind, ReadResult};
use crate::errors::{Error, Result};
use crate::limits;
use crate::notelocks::LockRules;
use crate::paths::RelPath;
use crate::storage::Vault;

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReadRequest {
    /// A folder or a Markdown note, relative to the vault root. Omit it to read
    /// the vault root.
    pub path: Option<String>,

    /// Note only: the 1-indexed first line to return (default 1).
    pub start_line: Option<u32>,
    /// Note only: how many lines to return (default 200, maximum 1000).
    pub max_lines: Option<u32>,
    /// Note only: the most content to return, in bytes (default 65536, maximum
    /// 262144).
    pub max_total_bytes: Option<u32>,

    /// Folder only: list everything beneath the folder, not just its children.
    #[serde(default)]
    pub recursive: bool,
    /// Folder only: how deep a recursive listing goes (default 4, maximum 16).
    pub max_depth: Option<u32>,
    /// Folder only: the `next_cursor` of a previous page; listing resumes
    /// strictly after it.
    pub cursor: Option<String>,
    /// Folder only: page size (default 50, maximum 200).
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ReadResponse {
    /// The vault-relative path that was read; empty for the vault root.
    pub path: String,
    /// `directory` for a folder, `document` for a note.
    pub kind: DocumentKind,
    /// Whether this folder or note is locked. A locked path can be read but
    /// not written, moved, or deleted until it is unlocked.
    pub locked: bool,
    /// The folder or note whose lock applies, when `locked` is true.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub locked_at: Option<String>,
    /// Present for a folder: one page of what it contains.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub listing: Option<Listing>,
    /// Present for a note: one bounded range of its content.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub document: Option<ReadResult>,
}

pub fn read(vault: &Vault, request: ReadRequest) -> Result<ReadResponse> {
    let path = match &request.path {
        Some(raw) => RelPath::parse(raw)?,
        None => RelPath::root(),
    };

    // A name that is not there at all is NOT_FOUND. A name that is there but
    // resolves outside the vault falls through to the read, which reports
    // PATH_OUTSIDE_VAULT — the caller learns which problem it has.
    if !vault.entry_exists(&path) {
        return Err(Error::not_found(format!("\"{path}\" does not exist"))
            .with_detail("path", path.as_str()));
    }

    // Hidden folders belong to other tools and `.trash/` to nobody; neither is
    // vault content, and no listing ever shows them.
    path.deny_hidden()?;

    let rules = LockRules::load(vault)?;
    let locked_at = rules.locked_at(&path);

    let (kind, listing, document) = if vault.is_dir(&path) {
        reject_note_options(&request)?;
        let listing = listing::list(
            vault,
            &path,
            ListingOptions {
                recursive: request.recursive,
                max_depth: request.max_depth,
                cursor: request.cursor.as_deref(),
                limit: request.limit,
            },
            &rules,
        )?;
        (DocumentKind::Directory, Some(listing), None)
    } else {
        reject_folder_options(&request)?;
        if !path.is_markdown() {
            return Err(Error::invalid_input(format!(
                "\"{path}\" is not a Markdown note; only folders and \".md\" files can be read"
            ))
            .with_detail("path", path.as_str()));
        }
        let document = read_range(
            vault,
            &path,
            limits::resolve_start_line(request.start_line)?,
            limits::resolve_max_lines(request.max_lines)?,
            limits::resolve_max_total_bytes(request.max_total_bytes)?,
        )?;
        (DocumentKind::Document, None, Some(document))
    };

    Ok(ReadResponse {
        path: path.to_string(),
        kind,
        locked: locked_at.is_some(),
        locked_at: locked_at.map(|at| at.to_string()),
        listing,
        document,
    })
}

/// A line range means nothing to a folder. Ignoring one would leave a caller
/// believing it had asked a question it had not.
fn reject_note_options(request: &ReadRequest) -> Result<()> {
    for (name, set) in [
        ("start_line", request.start_line.is_some()),
        ("max_lines", request.max_lines.is_some()),
        ("max_total_bytes", request.max_total_bytes.is_some()),
    ] {
        if set {
            return Err(Error::invalid_input(format!(
                "{name} applies to a note, and this path is a folder"
            ))
            .with_detail("parameter", name));
        }
    }
    Ok(())
}

fn reject_folder_options(request: &ReadRequest) -> Result<()> {
    for (name, set) in [
        ("recursive", request.recursive),
        ("max_depth", request.max_depth.is_some()),
        ("cursor", request.cursor.is_some()),
        ("limit", request.limit.is_some()),
    ] {
        if set {
            return Err(Error::invalid_input(format!(
                "{name} applies to a folder, and this path is a note"
            ))
            .with_detail("parameter", name));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::ErrorCode;
    use camino::Utf8PathBuf;

    fn vault() -> (tempfile::TempDir, Vault) {
        let dir = tempfile::tempdir().unwrap();
        let root = Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).unwrap();
        let vault = Vault::open(&root).unwrap();
        (dir, vault)
    }

    fn write(vault: &Vault, path: &str, content: &[u8]) {
        let path = RelPath::parse(path).unwrap();
        vault.create_dir_all(&path.parent()).unwrap();
        vault.atomic_write(&path, content).unwrap();
    }

    fn at(path: &str) -> ReadRequest {
        ReadRequest {
            path: Some(path.to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn a_note_comes_back_as_a_bounded_range() {
        let (_tmp, vault) = vault();
        write(&vault, "projects/b.md", b"beta\n");

        let response = read(&vault, at("projects/b.md")).unwrap();
        assert_eq!(response.kind, DocumentKind::Document);
        assert!(response.listing.is_none());
        let document = response.document.unwrap();
        assert_eq!(document.content, "beta\n");
        assert!(document.complete);
    }

    #[test]
    fn no_path_reads_the_vault_root_as_a_folder() {
        let (_tmp, vault) = vault();
        write(&vault, "a.md", b"x");

        let response = read(&vault, ReadRequest::default()).unwrap();
        assert_eq!(response.kind, DocumentKind::Directory);
        assert_eq!(response.path, "");
        assert!(!response.locked);
        let listing = response.listing.unwrap();
        assert_eq!(listing.entries[0].path, "a.md");
    }

    #[test]
    fn line_ranges_follow_continuation() {
        let (_tmp, vault) = vault();
        write(&vault, "a.md", b"1\n2\n3\n4\n");

        let response = read(
            &vault,
            ReadRequest {
                start_line: Some(3),
                max_lines: Some(1),
                ..at("a.md")
            },
        )
        .unwrap();
        let document = response.document.unwrap();
        assert_eq!(document.content, "3\n");
        assert_eq!(document.next_line, Some(4));
    }

    #[test]
    fn a_note_that_can_never_fit_the_budget_is_an_error() {
        let (_tmp, vault) = vault();
        write(&vault, "a.md", b"aaaaaaaaaaaaaaaaaaaa\n");

        let err = read(
            &vault,
            ReadRequest {
                max_total_bytes: Some(8),
                ..at("a.md")
            },
        )
        .unwrap_err();
        assert_eq!(err.code, ErrorCode::LimitExceeded);
    }

    #[test]
    fn options_for_the_other_kind_are_refused_rather_than_ignored() {
        let (_tmp, vault) = vault();
        write(&vault, "projects/a.md", b"x\n");

        let err = read(
            &vault,
            ReadRequest {
                max_lines: Some(5),
                ..at("projects")
            },
        )
        .unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidInput);
        assert_eq!(err.details["parameter"], "max_lines");

        let err = read(
            &vault,
            ReadRequest {
                recursive: true,
                ..at("projects/a.md")
            },
        )
        .unwrap_err();
        assert_eq!(err.details["parameter"], "recursive");
    }

    #[test]
    fn traversal_out_of_the_vault_is_refused() {
        let (_tmp, vault) = vault();
        for path in ["../escape.md", "projects/../../escape.md"] {
            let err = read(&vault, at(path)).unwrap_err();
            assert_eq!(err.code, ErrorCode::InvalidInput, "{path}");
        }
    }

    #[test]
    fn hidden_folders_and_the_trash_are_not_readable() {
        let (_tmp, vault) = vault();
        write(&vault, ".trash/gone.md", b"x\n");
        for path in [".trash", ".trash/gone.md"] {
            assert_eq!(read(&vault, at(path)).unwrap_err().code, ErrorCode::InvalidInput);
        }
    }

    #[test]
    fn non_markdown_files_are_rejected() {
        let (_tmp, vault) = vault();
        write(&vault, "image.png", b"x");
        assert_eq!(read(&vault, at("image.png")).unwrap_err().code, ErrorCode::InvalidInput);
        assert_eq!(read(&vault, at("absent.md")).unwrap_err().code, ErrorCode::NotFound);
    }

    #[test]
    fn a_read_reports_the_lock_and_where_it_comes_from() {
        let (_tmp, vault) = vault();
        write(&vault, "projects/a.md", b"x\n");
        let mut rules = LockRules::default();
        rules.set(&RelPath::parse("projects").unwrap(), true);
        rules.save(&vault).unwrap();

        let response = read(&vault, at("projects/a.md")).unwrap();
        assert!(response.locked);
        assert_eq!(response.locked_at.as_deref(), Some("projects"));

        let root = read(&vault, ReadRequest::default()).unwrap();
        assert!(!root.locked);
        assert!(root.listing.unwrap().entries.iter().all(|e| e.locked));
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_out_of_the_vault_is_named_as_an_escape_not_a_missing_file() {
        let (tmp, vault) = vault();
        let outside = tmp.path().parent().unwrap().join("heimdall-read-secret.md");
        std::fs::write(&outside, b"secret\n").unwrap();
        std::os::unix::fs::symlink(&outside, tmp.path().join("escape.md")).unwrap();

        let err = read(&vault, at("escape.md")).unwrap_err();
        assert_eq!(err.code, ErrorCode::PathOutsideVault);
        assert!(!err.message.contains("secret"), "{}", err.message);

        std::fs::remove_file(outside).unwrap();
    }
}

//! `write_document` — replace or create one ordinary note (SPEC §15).
//!
//! A client operation: the desktop calls it through the shell CLI and the MCP
//! surface does not expose it (SPEC §2, §9). A human editing their own vault in
//! Heimdall's own application needs to save; an AI client still does not.
//!
//! Deliberately no `JsonSchema` derive, following `create_vault`. `rmcp` builds
//! a tool's schemas from its request and response types, so a type without one
//! cannot be given a tool without a compile error — the nine-tool surface stops
//! being a promise and becomes a property the type system checks.

use serde::{Deserialize, Serialize};

use crate::commands::types::explicit_option;
use crate::errors::{Error, Result};
use crate::limits;
use crate::paths::RelPath;
use crate::revisions::Revision;
use crate::storage::Vault;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WriteDocumentRequest {
    /// Vault-relative Markdown path. Never inside `aios/` or a hidden folder.
    pub path: String,
    pub content: String,
    /// The revision from the latest read. Must be explicit `null` to create a
    /// new note; omitting the field entirely is an error.
    #[serde(default, deserialize_with = "explicit_option")]
    pub expected_revision: Option<Option<Revision>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WriteDocumentResponse {
    pub path: String,
    pub new_revision: Revision,
    pub size_bytes: usize,
    pub created: bool,
}

pub fn write_document(
    vault: &Vault,
    request: WriteDocumentRequest,
) -> Result<WriteDocumentResponse> {
    vault.ensure_initialized()?;

    let path = RelPath::parse_markdown(&request.path)?;
    path.deny_aios()?;
    path.deny_hidden()?;

    let bytes = request.content.as_bytes();
    limits::check_write_size(bytes.len(), limits::DOCUMENT_MAX_BYTES, "document")?;

    // The folder must already exist. Creating it here would mean one mistyped
    // path scatters directories through the vault, and the client has
    // `create_folder` for the case where it genuinely wants a new one.
    let parent = path.parent();
    if !parent.is_root() && !vault.is_dir(&parent) {
        return Err(Error::not_found(format!(
            "folder \"{parent}\" does not exist; create it first"
        ))
        .with_detail("path", parent.as_str()));
    }

    let Some(expected_revision) = request.expected_revision else {
        return Err(Error::invalid_input(
            "expected_revision is required; pass the revision from the latest read, \
             or explicit null to create a new note",
        )
        .with_detail("parameter", "expected_revision"));
    };

    let (new_revision, created) = vault.with_file_lock(&path, || {
        let exists = vault.is_file(&path);

        match (&expected_revision, exists) {
            (Some(expected), true) => {
                let current = Revision::of_bytes(&vault.read(&path)?);
                if *expected != current {
                    // The current revision goes back so the editor can offer a
                    // reload; the current content does not, because the caller
                    // asked to write, not to read.
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
            (None, false) => {}
        }

        Ok((vault.atomic_write(&path, bytes)?, !exists))
    })?;

    Ok(WriteDocumentResponse {
        path: path.to_string(),
        new_revision,
        size_bytes: bytes.len(),
        created,
    })
}

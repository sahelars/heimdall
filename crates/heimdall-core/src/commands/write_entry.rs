//! `write_entry` — replace one entry, keeping Heimdall's frontmatter (SPEC §15).
//!
//! A client operation; no `JsonSchema` derive, so it cannot become an MCP tool.
//! An AI client creates entries and never edits them; a human fixing the wording
//! of a summary they can already see is a different act.
//!
//! SPEC §6 gives Heimdall exclusive ownership of entry frontmatter, and
//! `create_entry` protects that by rejecting caller frontmatter outright. The
//! invariant it is actually protecting is narrower than the rule: *`created_at`
//! and `type` are what Heimdall wrote, and always agree with the filename and the
//! folder.* For an edit the correct rule follows the invariant instead of the
//! blanket ban — the incoming content must carry byte-identical values for both
//! owned fields, and every other key is the user's to add, change, or remove.
//! So a human can give an entry `tags:` or a `links:` list from Obsidian's
//! property editor, and cannot reassign its identity.

use serde::{Deserialize, Serialize};

use crate::commands::frontmatter;
use crate::commands::types::EntryKind;
use crate::errors::{Error, Result};
use crate::limits;
use crate::paths::BaseName;
use crate::revisions::Revision;
use crate::storage::Vault;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WriteEntryRequest {
    /// Selects the folder; must match where the entry actually lives.
    pub kind: EntryKind,
    /// A bare filename from `list_entries`, never a path.
    pub id: String,
    /// The complete replacement file, frontmatter block included.
    pub content: String,
    pub expected_revision: Revision,
}

#[derive(Debug, Clone, Serialize)]
pub struct WriteEntryResponse {
    pub id: String,
    pub path: String,
    pub kind: EntryKind,
    pub new_revision: Revision,
    pub size_bytes: usize,
}

pub fn write_entry(vault: &Vault, request: WriteEntryRequest) -> Result<WriteEntryResponse> {
    vault.ensure_initialized()?;

    let id = BaseName::parse_markdown(&request.id, "id")?;
    let path = request.kind.dir().join(id.as_str());

    let bytes = request.content.as_bytes();
    limits::check_write_size(bytes.len(), limits::ENTRY_CONTENT_MAX_BYTES, "entry")?;

    let incoming = frontmatter::owned(&request.content);

    // Whatever the content declares must match the folder, so a hand-made entry
    // cannot be edited into the wrong kind either.
    if let Some(declared) = incoming.as_ref().and_then(|block| block.kind.as_deref()) {
        if declared != request.kind.as_str() {
            return Err(Error::invalid_input(format!(
                "type is \"{declared}\" but this entry lives in the {} folder",
                request.kind
            ))
            .with_detail("parameter", "type"));
        }
    }

    let new_revision = vault.with_file_lock(&path, || {
        if !vault.is_file(&path) {
            return Err(Error::not_found(format!(
                "no {} entry with id \"{id}\"",
                request.kind
            ))
            .with_detail("kind", request.kind.as_str())
            .with_detail("id", id.as_str()));
        }

        let stored_bytes = vault.read(&path)?;
        let current = Revision::of_bytes(&stored_bytes);
        if request.expected_revision != current {
            return Err(Error::revision_conflict(format!(
                "\"{path}\" changed since it was read; re-read it and retry"
            ))
            .with_detail("path", path.as_str())
            .with_detail("current_revision", current.as_str()));
        }

        let stored_text = std::str::from_utf8(&stored_bytes).map_err(|_| {
            Error::io_error(format!("\"{path}\" is not valid UTF-8 and cannot be edited"))
                .with_detail("path", path.as_str())
        })?;

        if let Some(stored) = frontmatter::owned(stored_text).filter(frontmatter::Owned::is_present)
        {
            let Some(incoming) = incoming.as_ref() else {
                return Err(Error::invalid_input(
                    "this entry's leading \"---\" block is Heimdall's; keep it, including \
                     created_at and type, and edit the body below it",
                )
                .with_detail("parameter", "content"));
            };
            if incoming.created_at != stored.created_at {
                return Err(Error::invalid_input(
                    "created_at is set when an entry is created and cannot be changed; it names \
                     the same instant as the filename",
                )
                .with_detail("parameter", "created_at"));
            }
            if incoming.kind != stored.kind {
                return Err(Error::invalid_input(
                    "type is set by Heimdall and always matches the folder the entry lives in",
                )
                .with_detail("parameter", "type"));
            }
        }

        vault.atomic_write(&path, bytes)
    })?;

    Ok(WriteEntryResponse {
        id: id.to_string(),
        path: path.to_string(),
        kind: request.kind,
        new_revision,
        size_bytes: bytes.len(),
    })
}

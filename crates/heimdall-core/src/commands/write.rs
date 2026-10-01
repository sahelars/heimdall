//! `write` — create or replace one note (SPEC §10).
//!
//! The one way anything is written into a note, whoever is writing: an agent
//! over MCP, a script at the shell, or the desktop editor saving. It never
//! overwrites silently:
//!
//! - with no `expected_revision`, it creates the note, and refuses if one is
//!   already there;
//! - with `expected_revision: null`, the same, said explicitly;
//! - with a revision, it replaces the note only if that is still what is on
//!   disk.
//!
//! A locked note, or a new note in a locked folder, is refused with `LOCKED`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::agents;
use crate::commands::types::explicit_option;
use crate::errors::{Error, Result};
use crate::limits;
use crate::notelocks::Locks;
use crate::paths::RelPath;
use crate::revisions::Revision;
use crate::storage::Vault;

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WriteRequest {
    /// The shared vault to write into, by its name — never a filesystem path.
    /// It may be left out only when exactly one vault is shared; otherwise the
    /// call is refused with the list of names.
    #[serde(default)]
    pub vault: Option<String>,
    /// The Markdown note to write, relative to the vault root. It must end in
    /// `.md`, and its folder must already exist.
    pub path: String,
    /// The complete new content of the note.
    pub content: String,
    /// The `revision` from the latest `read` of this note, to replace it. Omit
    /// it (or pass null) to create a new note; that never replaces one.
    #[serde(default, deserialize_with = "explicit_option")]
    #[schemars(with = "Option<Revision>")]
    pub expected_revision: Option<Option<Revision>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WriteResponse {
    /// The name of the vault that was written.
    pub vault: String,
    pub path: String,
    /// The revision of the bytes now on disk. Pass it as `expected_revision`
    /// to write this note again.
    pub new_revision: Revision,
    pub size_bytes: usize,
    /// True when the note did not exist before this write.
    pub created: bool,
}

pub fn write(vault: &Vault, request: WriteRequest) -> Result<WriteResponse> {
    let name = agents::confirm_name(vault, request.vault.as_deref())?;
    let path = RelPath::parse_markdown(&request.path)?;
    path.deny_hidden()?;

    let bytes = request.content.as_bytes();
    limits::check_write_size(bytes.len(), limits::DOCUMENT_MAX_BYTES, "document")?;

    // The folder must already exist. Creating it here would mean one mistyped
    // path scatters directories through the vault.
    let parent = path.parent();
    if !parent.is_root() && !vault.is_dir(&parent) {
        return Err(Error::not_found(format!(
            "folder \"{parent}\" does not exist; create it first"
        ))
        .with_detail("path", parent.as_str()));
    }

    // Absent and explicit null both mean "create": neither can replace a note.
    let expected_revision = request.expected_revision.flatten();

    let (new_revision, created) = vault.with_write_lock(&path, || {
        let exists = vault.is_file(&path);

        // Checked under the write lock, so an unlock or a lock cannot land
        // between this decision and the rename below.
        let rules = Locks::load(vault)?;
        if exists {
            rules.deny_write(&path)?;
        } else {
            rules.deny_change_in(&parent, &path)?;
        }

        match (&expected_revision, exists) {
            (Some(expected), true) => {
                let current = Revision::of_bytes(&vault.read(&path)?);
                if *expected != current {
                    // The current revision goes back so the caller can re-read;
                    // the current content does not, because it asked to write.
                    return Err(Error::revision_conflict(format!(
                        "\"{path}\" changed since it was read; read it again and retry"
                    ))
                    .with_detail("path", path.as_str())
                    .with_detail("current_revision", current.as_str()));
                }
            }
            (Some(_), false) => {
                return Err(Error::not_found(format!(
                    "\"{path}\" does not exist; omit expected_revision to create it"
                ))
                .with_detail("path", path.as_str()));
            }
            (None, true) => {
                let current = Revision::of_bytes(&vault.read(&path)?);
                return Err(Error::revision_conflict(format!(
                    "\"{path}\" already exists; read it and pass its revision to replace it"
                ))
                .with_detail("path", path.as_str())
                .with_detail("current_revision", current.as_str()));
            }
            (None, false) => {}
        }

        Ok((vault.atomic_write(&path, bytes)?, !exists))
    })?;

    Ok(WriteResponse {
        vault: name,
        path: path.to_string(),
        new_revision,
        size_bytes: bytes.len(),
        created,
    })
}

//! `move_path` — rename or relocate one ordinary note or folder (SPEC §15).
//!
//! A client operation; no `JsonSchema` derive, so it cannot become an MCP tool.
//!
//! Inbound `[[wikilinks]]` are deliberately **not** rewritten. Doing it here
//! would make one command an unbounded whole-vault read (SPEC §8) followed by an
//! unbounded multi-file write with no `expected_revision` on any of the files it
//! touched — silently editing notes the user never opened, which is exactly what
//! the revision contract exists to prevent. The client already has what it
//! needs: `link_graph` gives it the reverse edges, so it can warn about the
//! links a rename will break and then rewrite them with ordinary
//! `write_document` calls, each guarded by that file's own revision.

use serde::{Deserialize, Serialize};

use crate::commands::types::DocumentKind;
use crate::errors::{Error, Result};
use crate::paths::{self, RelPath};
use crate::storage::Vault;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MovePathRequest {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct MovePathResponse {
    pub from: String,
    pub to: String,
    pub kind: DocumentKind,
}

pub fn move_path(vault: &Vault, request: MovePathRequest) -> Result<MovePathResponse> {
    vault.ensure_initialized()?;

    let from = validate(&request.from, "from")?;
    let to = validate(&request.to, "to")?;

    if from.as_str() == to.as_str() {
        return Err(Error::invalid_input("the source and destination are the same path")
            .with_detail("path", from.as_str()));
    }

    let is_dir = vault.is_dir(&from);
    if !is_dir && !vault.is_file(&from) {
        return Err(Error::not_found(format!("\"{from}\" does not exist"))
            .with_detail("path", from.as_str()));
    }

    // Managed structure has to stay where `ensure_initialized` looks for it, or
    // the next call finds a vault that no longer opens. Note this also covers
    // `aios/` itself, and therefore anything containing it.
    if paths::is_structural(&from) {
        return Err(Error::invalid_input(format!(
            "\"{from}\" is part of the managed vault structure and cannot be moved"
        ))
        .with_detail("path", from.as_str()));
    }

    // Moving a folder inside itself detaches the subtree from the vault. The
    // filesystem reports it as EINVAL, which would surface as an opaque
    // IO_ERROR, so name it here.
    if is_dir && is_inside(&to, &from) {
        return Err(Error::invalid_input(format!(
            "\"{to}\" is inside \"{from}\" and cannot be its own destination"
        ))
        .with_detail("path", to.as_str()));
    }

    // A note keeps its extension across a move. Every read operation reaches
    // only `.md` files, so renaming one to `notes.txt` would make it unreachable
    // through Heimdall without ever deleting it.
    if !is_dir && !to.is_markdown() {
        return Err(Error::invalid_input(format!(
            "\"{to}\" must end in \".md\"; Heimdall exposes no operation that can read anything else"
        ))
        .with_detail("parameter", "to"));
    }
    if is_dir && to.is_markdown() {
        return Err(Error::invalid_input(format!(
            "\"{from}\" is a folder, so \"{to}\" must not end in \".md\""
        ))
        .with_detail("parameter", "to"));
    }

    let parent = to.parent();
    if !parent.is_root() && !vault.is_dir(&parent) {
        return Err(Error::not_found(format!(
            "folder \"{parent}\" does not exist; create it first"
        ))
        .with_detail("path", parent.as_str()));
    }

    // The destination's lock is what makes "does not already exist" and "rename
    // onto it" one decision rather than two.
    vault.with_file_lock(&to, || vault.rename_no_replace(&from, &to))?;

    if !is_dir {
        vault.discard_lock_sidecar(&from);
    }

    Ok(MovePathResponse {
        from: from.to_string(),
        to: to.to_string(),
        kind: if is_dir {
            DocumentKind::Directory
        } else {
            DocumentKind::Document
        },
    })
}

/// Both ends of a move must be ordinary, visible vault content.
///
/// Crossing the `aios/` boundary is refused in **both** directions, and that is
/// the load-bearing rule. Dropping a note into `aios/conversations/` would
/// manufacture a file the entry contract governs — Heimdall-owned frontmatter, a
/// UTC filename `parse_entry_stem` can read, a `type` matching the folder (SPEC
/// §6) — with none of those properties, and `list_entries` would then report a
/// `created_at` silently falling back to the file's mtime. Moving one out turns
/// managed content into a document that nothing rewrites. A rename cannot
/// perform either transformation, so it must not pretend to.
fn validate(raw: &str, parameter: &str) -> Result<RelPath> {
    let path = RelPath::parse_file(raw).map_err(|err| err.with_detail("parameter", parameter))?;
    if path.is_in_aios() {
        return Err(Error::invalid_input(format!(
            "\"{path}\" is inside \"aios/\"; protected content cannot be moved in or out of the \
             managed tree"
        ))
        .with_detail("parameter", parameter)
        .with_detail("path", path.as_str()));
    }
    path.deny_hidden()?;
    Ok(path)
}

/// Whether `inner` lies at or beneath `outer`, compared by whole components.
fn is_inside(inner: &RelPath, outer: &RelPath) -> bool {
    let mut outer_components = outer.components();
    let mut inner_components = inner.components();
    loop {
        match (outer_components.next(), inner_components.next()) {
            (None, _) => return true,
            (Some(_), None) => return false,
            (Some(a), Some(b)) if a == b => continue,
            (Some(_), Some(_)) => return false,
        }
    }
}

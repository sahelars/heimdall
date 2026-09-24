//! `move_path` — rename or relocate one ordinary note or folder (SPEC §15).
//!
//! A client operation; no `JsonSchema` derive, so it cannot become an MCP tool.
//!
//! Inbound `[[wikilinks]]` are deliberately **not** rewritten here. `relink` is
//! that work, and it is a separate operation on purpose: a move is one rename,
//! and folding an unbounded whole-vault read (SPEC §8) and a multi-file write
//! into it would leave a command that half succeeded with no way to say which
//! half. Keeping them apart means a move either happened or did not, and what
//! the rewrite did afterwards is reported on its own terms.
//!
//! A lock fixes a note's place as well as its bytes, so nothing locked moves:
//! not the path, not anything inside it, and not into or out of a locked
//! folder. Lock rules travel with what moved.

use serde::{Deserialize, Serialize};

use crate::commands::types::DocumentKind;
use crate::errors::{Error, Result};
use crate::notelocks::LockRules;
use crate::paths::RelPath;
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

    // Moving a folder inside itself detaches the subtree from the vault. The
    // filesystem reports it as EINVAL, which would surface as an opaque
    // IO_ERROR, so name it here.
    if is_dir && to.is_within(&from) {
        return Err(Error::invalid_input(format!(
            "\"{to}\" is inside \"{from}\" and cannot be its own destination"
        ))
        .with_detail("path", to.as_str()));
    }

    // A note keeps its extension across a move. `read` reaches only `.md`
    // files, so renaming one to `notes.txt` would make it unreachable
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

    // The write lock is what makes "does not already exist" and "rename onto it"
    // one decision rather than two.
    vault.with_write_lock(&to, || {
        let mut rules = LockRules::load(vault)?;
        rules.deny_subtree(&from)?;
        rules.deny_change_in(&from.parent(), &from)?;
        rules.deny_change_in(&parent, &to)?;

        vault.rename_no_replace(&from, &to)?;
        let before = rules.clone();
        rules.carry(&from, &to);
        if rules != before {
            rules.save(vault)?;
        }
        Ok(())
    })?;

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

/// Both ends of a move must be visible vault content.
fn validate(raw: &str, parameter: &str) -> Result<RelPath> {
    let path = RelPath::parse_file(raw).map_err(|err| err.with_detail("parameter", parameter))?;
    path.deny_hidden()?;
    Ok(path)
}

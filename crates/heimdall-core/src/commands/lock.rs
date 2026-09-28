//! `lock` and `unlock` — make part of a vault read-only, or writable again
//! (SPEC §10).
//!
//! The target is the vault root, a folder, or a note. Locking a folder locks
//! everything in it, and so does unlocking — including notes that had been
//! given their own rule — after which a single note inside can be set apart
//! again. The rules themselves are in [`crate::notelocks`].
//!
//! Shell-only: a lock is the user's decision, so neither type derives
//! `JsonSchema` and neither can be given an MCP tool (SPEC §9).

use serde::{Deserialize, Serialize};

use crate::commands::types::DocumentKind;
use crate::errors::{Error, Result};
use crate::notelocks::LockRules;
use crate::paths::RelPath;
use crate::storage::Vault;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LockRequest {
    /// A folder or a Markdown note, relative to the vault root. Omit it for the
    /// whole vault.
    pub path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockResponse {
    /// The vault-relative path; empty for the whole vault.
    pub path: String,
    pub kind: DocumentKind,
    /// Whether the path is locked now.
    pub locked: bool,
    /// False when the path was already in this state and nothing was stored.
    pub changed: bool,
}

pub fn lock(vault: &Vault, request: LockRequest) -> Result<LockResponse> {
    set(vault, request, true)
}

pub fn unlock(vault: &Vault, request: LockRequest) -> Result<LockResponse> {
    set(vault, request, false)
}

fn set(vault: &Vault, request: LockRequest, locked: bool) -> Result<LockResponse> {
    let path = match &request.path {
        Some(raw) => RelPath::parse(raw)?,
        None => RelPath::root(),
    };
    path.deny_hidden()?;

    let kind = if vault.is_dir(&path) {
        DocumentKind::Directory
    } else if vault.is_file(&path) && path.is_markdown() {
        DocumentKind::Document
    } else if vault.entry_exists(&path) {
        return Err(Error::invalid_input(format!(
            "\"{path}\" is not a folder or a Markdown note"
        ))
        .with_detail("path", path.as_str()));
    } else {
        return Err(Error::not_found(format!("\"{path}\" does not exist"))
            .with_detail("path", path.as_str()));
    };

    let changed = vault.with_write_lock(&path, || {
        let loaded = LockRules::load(vault)?;
        let mut rules = loaded.clone();
        rules.prune(vault);
        let changed = rules.set(&path, locked);
        if rules != loaded {
            rules.save(vault)?;
        }
        Ok(changed)
    })?;

    Ok(LockResponse {
        path: path.to_string(),
        kind,
        locked,
        changed,
    })
}

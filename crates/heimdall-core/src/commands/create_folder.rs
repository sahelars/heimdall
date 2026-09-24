//! `create_folder` — make one folder for ordinary notes (SPEC §15).
//!
//! A client operation; no `JsonSchema` derive, so it cannot become an MCP tool.
//!
//! A folder cannot be made inside a locked one: a lock fixes what a folder
//! holds, not only the bytes of the notes in it.

use serde::{Deserialize, Serialize};

use crate::errors::{Error, Result};
use crate::notelocks::LockRules;
use crate::paths::RelPath;
use crate::storage::Vault;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateFolderRequest {
    /// Vault-relative path of the folder to create.
    pub path: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CreateFolderResponse {
    pub path: String,
    /// The directories actually made, outermost first, mirroring
    /// `CreateVaultResponse`. Missing parents are created along the way.
    pub created: Vec<String>,
}

pub fn create_folder(vault: &Vault, request: CreateFolderRequest) -> Result<CreateFolderResponse> {
    let path = RelPath::parse_file(&request.path)?;
    path.deny_hidden()?;

    if vault.entry_exists(&path) {
        return Err(Error::already_exists(format!("\"{path}\" already exists"))
            .with_detail("path", path.as_str()));
    }

    // Missing parents are made too, so the folder that must be unlocked is the
    // deepest one that already exists.
    let mut existing = path.parent();
    while !existing.is_root() && !vault.is_dir(&existing) {
        existing = existing.parent();
    }
    let created = vault.with_write_lock(&path, || {
        LockRules::load(vault)?.deny_change_in(&existing, &path)?;
        vault.create_dir_all(&path)
    })?;

    Ok(CreateFolderResponse {
        path: path.to_string(),
        created: created.iter().map(RelPath::to_string).collect(),
    })
}

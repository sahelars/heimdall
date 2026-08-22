//! `create_folder` — make one folder for ordinary notes (SPEC §15).
//!
//! A client operation; no `JsonSchema` derive, so it cannot become an MCP tool.
//!
//! `aios/` is refused: its structure is Heimdall's, and `list_memories` and
//! `list_entries` are both flat, so a subfolder created there would be invisible
//! to every operation that reads the protected tree.

use serde::{Deserialize, Serialize};

use crate::errors::{Error, Result};
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
    vault.ensure_initialized()?;

    let path = RelPath::parse_file(&request.path)?;
    path.deny_aios()?;
    path.deny_hidden()?;

    if vault.entry_exists(&path) {
        return Err(Error::already_exists(format!("\"{path}\" already exists"))
            .with_detail("path", path.as_str()));
    }

    let created = vault.create_dir_all(&path)?;

    Ok(CreateFolderResponse {
        path: path.to_string(),
        created: created.iter().map(RelPath::to_string).collect(),
    })
}

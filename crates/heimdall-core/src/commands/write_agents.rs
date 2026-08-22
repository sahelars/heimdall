//! `write_agents` — replace the vault's agent instructions (SPEC §15).
//!
//! A client operation; no `JsonSchema` derive, so it cannot become an MCP tool.
//! An AI client reading its own instructions is `read_agents`; an AI client
//! rewriting them is not a feature.
//!
//! The path is resolved through `Vault::agents_file`, never through the
//! `AGENTS_FILE_CANDIDATES` constant, and that distinction is load-bearing.
//! Both `aios/agents.md` and `aios/AGENTS.md` are valid spellings (SPEC §6) and
//! the shipped template uses the uppercase one. A write aimed at the constant's
//! first entry would, on a case-sensitive filesystem, create a *second* file —
//! and since `agents_file()` prefers the lowercase spelling, every later read
//! would return the new file and quietly orphan the user's real instructions.

use serde::{Deserialize, Serialize};

use crate::errors::{Error, Result};
use crate::limits;
use crate::revisions::Revision;
use crate::storage::Vault;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WriteAgentsRequest {
    pub content: String,
    /// Required. The file always exists in an initialized vault, so there is no
    /// create case and no explicit-null spelling to distinguish.
    pub expected_revision: Revision,
}

#[derive(Debug, Clone, Serialize)]
pub struct WriteAgentsResponse {
    pub path: String,
    pub new_revision: Revision,
    pub size_bytes: usize,
}

pub fn write_agents(vault: &Vault, request: WriteAgentsRequest) -> Result<WriteAgentsResponse> {
    vault.ensure_initialized()?;

    let path = vault
        .agents_file()
        .ok_or_else(|| Error::not_found("this vault has no agent instructions file"))?;

    let bytes = request.content.as_bytes();
    limits::check_write_size(bytes.len(), limits::AGENTS_MAX_BYTES, "agent instructions")?;

    let new_revision = vault.with_file_lock(&path, || {
        // The file existed a moment ago; if it does not now, it vanished between
        // `ensure_initialized` and the lock.
        if !vault.is_file(&path) {
            return Err(Error::not_initialized(
                "the agent instructions file is missing; run \"heimdall create\" against this vault",
            ));
        }

        let current = Revision::of_bytes(&vault.read(&path)?);
        if request.expected_revision != current {
            return Err(Error::revision_conflict(format!(
                "\"{path}\" changed since it was read; re-read it and retry"
            ))
            .with_detail("path", path.as_str())
            .with_detail("current_revision", current.as_str()));
        }

        vault.atomic_write(&path, bytes)
    })?;

    Ok(WriteAgentsResponse {
        path: path.to_string(),
        new_revision,
        size_bytes: bytes.len(),
    })
}

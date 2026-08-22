//! `read_agents` — bounded read of the vault's agent instructions (SPEC §10).

use schemars::JsonSchema;
use serde::Deserialize;

use crate::commands::read_range::read_range;
use crate::commands::types::ReadResult;
use crate::errors::{Error, Result};
use crate::limits;
use crate::storage::Vault;

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReadAgentsRequest {
    pub start_line: Option<u32>,
    pub max_lines: Option<u32>,
}

pub fn read_agents(vault: &Vault, request: ReadAgentsRequest) -> Result<ReadResult> {
    vault.ensure_initialized()?;

    let start_line = limits::resolve_start_line(request.start_line)?;
    let max_lines = limits::resolve_max_lines(request.max_lines)?;
    let path = vault
        .agents_file()
        .ok_or_else(|| Error::not_found("this vault has no agent instructions file"))?;

    read_range(
        vault,
        &path,
        start_line,
        max_lines,
        limits::READ_TOTAL_BYTES_MAX,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::ErrorCode;
    use crate::paths::RelPath;
    use crate::template;
    use camino::Utf8PathBuf;

    fn scaffolded() -> (tempfile::TempDir, Vault) {
        let dir = tempfile::tempdir().unwrap();
        let root = Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).unwrap();
        let vault = Vault::open(&root).unwrap();
        template::scaffold_full(&vault).unwrap();
        (dir, vault)
    }

    #[test]
    fn reads_the_agent_instructions_from_the_template() {
        let (_tmp, vault) = scaffolded();
        let result = read_agents(&vault, ReadAgentsRequest::default()).unwrap();

        assert!(result.path.starts_with("aios/"));
        assert!(result.complete);
        assert!(!result.content.is_empty());
    }

    #[test]
    fn honors_line_bounds_like_every_other_read() {
        let (_tmp, vault) = scaffolded();
        let path = vault.agents_file().unwrap();
        vault.atomic_write(&path, b"one\ntwo\nthree\n").unwrap();

        let result = read_agents(
            &vault,
            ReadAgentsRequest {
                start_line: Some(2),
                max_lines: Some(1),
            },
        )
        .unwrap();

        assert_eq!(result.content, "two\n");
        assert_eq!(result.next_line, Some(3));
    }

    #[test]
    fn an_uninitialized_vault_says_so_before_anything_else() {
        let dir = tempfile::tempdir().unwrap();
        let root = Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).unwrap();
        let vault = Vault::open(&root).unwrap();

        let err = read_agents(&vault, ReadAgentsRequest::default()).unwrap_err();
        assert_eq!(err.code, ErrorCode::NotInitialized);
    }

    #[test]
    fn either_agents_filename_is_readable() {
        for name in ["aios/agents.md", "aios/AGENTS.md"] {
            let (_tmp, vault) = scaffolded();
            // Replace whichever spelling the template shipped with this one.
            let existing = vault.agents_file().unwrap();
            let target = RelPath::parse(name).unwrap();
            if existing != target {
                vault.atomic_write(&target, b"instructions\n").unwrap();
            }
            let result = read_agents(&vault, ReadAgentsRequest::default()).unwrap();
            assert!(!result.content.is_empty(), "{name}");
        }
    }
}

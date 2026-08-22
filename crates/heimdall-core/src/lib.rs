//! Heimdall core: the single implementation of domain rules and filesystem
//! behavior for Obsidian-compatible Markdown vaults (SPEC §4).
//!
//! Every adapter — the shell CLI today, the MCP server and desktop bridge
//! later — translates into the operations exposed here. Nothing above this
//! crate touches the filesystem.

/// The version of this crate, reported by `heimdall --version --json`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub mod commands;
pub mod errors;
pub mod limits;
pub mod paths;
pub mod revisions;
pub mod storage;
pub mod template;
pub mod timestamps;

pub use commands::*;
pub use errors::{Error, ErrorCode, Result};
pub use paths::{BaseName, RelPath};
pub use revisions::Revision;
pub use storage::Vault;

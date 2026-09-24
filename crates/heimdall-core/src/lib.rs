//! Heimdall core: the single implementation of domain rules and filesystem
//! behavior for Markdown vaults (SPEC §4).
//!
//! Every adapter — the shell CLI, the MCP server, and through the CLI the
//! desktop bridge — translates into the operations exposed here. Nothing above
//! this crate touches the filesystem.

/// The version of this crate, reported by `heimdall --version --json`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub mod appdata;
pub mod commands;
pub mod errors;
pub mod limits;
pub mod notelocks;
pub mod paths;
pub mod registry;
pub mod revisions;
pub mod storage;
pub mod template;
pub mod timestamps;

pub use commands::*;
pub use errors::{Error, ErrorCode, Result};
pub use paths::RelPath;
pub use revisions::Revision;
pub use storage::Vault;

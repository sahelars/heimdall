//! Types shared across operations.

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};

use crate::revisions::Revision;

/// One bounded range of one file (SPEC §8).
///
/// `complete` and `next_line` together mean a caller never has to guess whether
/// it has the whole file: partial content is always labelled as partial.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ReadResult {
    pub path: String,
    pub content: String,
    pub start_line: usize,
    pub end_line: usize,
    pub next_line: Option<usize>,
    pub complete: bool,
    pub size_bytes: u64,
    pub revision: Revision,
}

/// Whether a path is a folder or a note.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DocumentKind {
    Directory,
    Document,
}

/// Distinguish an absent field from an explicit `null` in a request.
///
/// Kept distinct at the edge so a request can be parsed faithfully; `write`
/// treats both as "create", since neither can ever replace a note.
pub(crate) fn explicit_option<'de, D, T>(
    deserializer: D,
) -> std::result::Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

//! Types shared across operations.

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};

use crate::paths::{self, RelPath};
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

/// What a `list_documents` result refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DocumentKind {
    Directory,
    Document,
}

/// Which memory a listing entry refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MemoryKind {
    Main,
    Extended,
}

/// The two V1 entry kinds. The value always matches the folder the entry lives
/// in and the `type` field of its frontmatter (SPEC §6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EntryKind {
    Conversation,
    Notification,
}

impl EntryKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Conversation => "conversation",
            Self::Notification => "notification",
        }
    }

    /// The folder this kind is stored in.
    pub fn dir(self) -> RelPath {
        let raw = match self {
            Self::Conversation => paths::CONVERSATIONS_DIR,
            Self::Notification => paths::NOTIFICATIONS_DIR,
        };
        RelPath::parse(raw).expect("entry directory constants are valid paths")
    }
}

impl std::fmt::Display for EntryKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Distinguish an absent field from an explicit `null` in a request.
///
/// Every replace-style write treats the two differently: `null` is a deliberate
/// "this file does not exist yet", while a missing field is a caller that forgot
/// the revision it read. Silently reading the second as a create would let a
/// stale editor clobber a file someone else had already changed.
pub(crate) fn explicit_option<'de, D, T>(
    deserializer: D,
) -> std::result::Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

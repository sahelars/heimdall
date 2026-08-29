//! `link_graph` — the whole vault's link structure in one call (SPEC §8, §15).
//!
//! A client operation, and the only one that reads the entire vault. It exists
//! because the desktop's graph pane, file tree, and backlinks panel all need the
//! same picture at once, and assembling it from bounded reads would cost one
//! process per ten notes.
//!
//! It covers `aios/` as well, which the ordinary document operations may not.
//! Three things make that admissible rather than a hole in the boundary:
//!
//! 1. **It is unreachable from MCP by construction.** These types derive no
//!    `JsonSchema`, so giving them a `#[tool]` would not compile. The boundary
//!    exists to stop an AI client reaching protected state through a
//!    general-purpose door; there is no door.
//! 2. **It returns no content, ever** — paths, titles, sizes, modification
//!    times, and link endpoints. That is strictly less than `list_memories` and
//!    `list_entries` already report about the same tree.
//! 3. **The caller is the human at the keyboard**, who can open every one of
//!    those files in any editor already. The same reasoning SPEC §7 applies to
//!    `heimdall create`.
//!
//! Bounds are caps rather than pagination, and hitting one is a truncated
//! success — the treatment `list_documents` gives its own scan guard, not an
//! error that throws away the work already done.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::commands::links;
use crate::commands::resolve::{self, Index};
use crate::errors::Result;
use crate::limits;
use crate::paths::RelPath;
use crate::storage::Vault;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LinkGraphRequest {
    /// Include the protected `aios/` tree. Defaults to true: the desktop's file
    /// tree shows it, so the graph must too.
    #[serde(default = "included")]
    pub include_aios: bool,
    /// How deep to descend. Defaults to and is capped at the recursion maximum.
    pub max_depth: Option<u32>,
}

fn included() -> bool {
    true
}

// Written out rather than derived: a derived `Default` would give
// `include_aios: false` and quietly disagree with the serde default above.
impl Default for LinkGraphRequest {
    fn default() -> Self {
        Self {
            include_aios: true,
            max_depth: None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct GraphNode {
    pub path: String,
    /// The filename stem — what a graph shows under the dot.
    pub title: String,
    pub in_aios: bool,
    pub size_bytes: u64,
    pub modified_at: String,
    /// False when this file was too large, unreadable, or past the scan budget,
    /// so its outgoing links are *absent* rather than known to be empty.
    pub scanned: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct GraphEdge {
    /// Index into `nodes`.
    pub from: usize,
    /// Index into `nodes`.
    pub to: usize,
    /// How many times `from` links to `to` — a natural edge weight.
    pub count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct UnresolvedLink {
    pub from: usize,
    /// The target exactly as written, before resolution.
    pub target: String,
    pub count: usize,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct GraphTruncation {
    pub node_cap_hit: bool,
    pub total_bytes_cap_hit: bool,
    /// Markdown files found but not carried as nodes.
    pub nodes_omitted: usize,
    /// Nodes present whose outgoing links were not extracted.
    pub files_unscanned: usize,
    pub scanned_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct LinkGraphResponse {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
    pub unresolved: Vec<UnresolvedLink>,
    pub truncated: GraphTruncation,
}

pub fn link_graph(vault: &Vault, request: LinkGraphRequest) -> Result<LinkGraphResponse> {
    vault.ensure_initialized()?;
    let max_depth = limits::resolve_graph_depth(request.max_depth)?;

    let mut found = Vec::new();
    let mut omitted = 0usize;
    resolve::walk(
        vault,
        &RelPath::root(),
        1,
        max_depth,
        request.include_aios,
        &mut found,
        &mut omitted,
    );

    // The same total order `list_documents` uses, and the reason two runs over
    // an unchanged vault produce byte-identical output.
    found.sort_by(|a, b| a.path.as_str().cmp(b.path.as_str()));

    let index = Index::build(found.iter().map(|note| &note.path));
    let mut nodes: Vec<GraphNode> = found
        .iter()
        .map(|note| GraphNode {
            path: note.path.to_string(),
            title: title_of(&note.path),
            in_aios: note.path.is_in_aios(),
            size_bytes: note.size_bytes,
            modified_at: note.modified_at.clone(),
            scanned: false,
        })
        .collect();

    let mut truncated = GraphTruncation {
        node_cap_hit: omitted > 0,
        nodes_omitted: omitted,
        ..Default::default()
    };

    let mut edges: BTreeMap<(usize, usize), usize> = BTreeMap::new();
    let mut unresolved: BTreeMap<(usize, String), usize> = BTreeMap::new();

    for (from, note) in found.iter().enumerate() {
        if note.size_bytes > limits::GRAPH_MAX_FILE_BYTES {
            truncated.files_unscanned += 1;
            continue;
        }
        if truncated.scanned_bytes + note.size_bytes > limits::GRAPH_MAX_TOTAL_BYTES {
            // Out of budget. Everything from here on stays a node without edges,
            // which the response says plainly rather than implying they have none.
            truncated.total_bytes_cap_hit = true;
            truncated.files_unscanned += found.len() - from;
            break;
        }

        let Ok(bytes) = vault.read(&note.path) else {
            // One unreadable file must not fail the whole index.
            truncated.files_unscanned += 1;
            continue;
        };
        let Ok(text) = std::str::from_utf8(&bytes) else {
            // A deliberate divergence from `read_range`, which reports invalid
            // UTF-8 as an error. Here it is one node without edges.
            truncated.files_unscanned += 1;
            continue;
        };

        truncated.scanned_bytes += note.size_bytes;
        nodes[from].scanned = true;

        for link in links::scan(text).into_iter().take(limits::GRAPH_MAX_LINKS_PER_FILE) {
            match index.resolve(&link, &note.path) {
                // A self-loop tells a reader nothing a graph can draw.
                Some(to) if to == from => {}
                Some(to) => *edges.entry((from, to)).or_default() += 1,
                None => *unresolved.entry((from, link.target)).or_default() += 1,
            }
        }
    }

    Ok(LinkGraphResponse {
        nodes,
        edges: edges
            .into_iter()
            .map(|((from, to), count)| GraphEdge { from, to, count })
            .collect(),
        unresolved: unresolved
            .into_iter()
            .map(|((from, target), count)| UnresolvedLink { from, target, count })
            .collect(),
        truncated,
    })
}

/// The filename without its extension.
fn title_of(path: &RelPath) -> String {
    let name = path.file_name().unwrap_or_default();
    name.rsplit_once('.')
        .map(|(stem, _)| stem)
        .filter(|stem| !stem.is_empty())
        .unwrap_or(name)
        .to_string()
}


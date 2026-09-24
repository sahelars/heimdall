//! `link_graph` — the whole vault's link structure in one call (SPEC §8, §15).
//!
//! A client operation, and the only one that reads the entire vault. It exists
//! because the desktop's graph pane, file tree, and backlinks panel all need the
//! same picture at once, and assembling it from bounded reads would cost one
//! process per ten notes.
//!
//! It is unreachable from MCP by construction: these types derive no
//! `JsonSchema`, so giving them a `#[tool]` would not compile. It returns no
//! content, ever — paths, titles, sizes, modification times, lock state, and
//! link endpoints.
//!
//! Bounds are caps rather than pagination, and hitting one is a truncated
//! success — the treatment a folder `read` gives its own scan guard, not an
//! error that throws away the work already done.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::commands::links;
use crate::commands::resolve::{self, Index};
use crate::errors::Result;
use crate::limits;
use crate::notelocks::LockRules;
use crate::paths::RelPath;
use crate::storage::Vault;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LinkGraphRequest {
    /// How deep to descend. Defaults to and is capped at the recursion maximum.
    pub max_depth: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GraphNode {
    pub path: String,
    /// The filename stem — what a graph shows under the dot.
    pub title: String,
    /// Whether the note is locked (read-only).
    pub locked: bool,
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
    let max_depth = limits::resolve_graph_depth(request.max_depth)?;
    let rules = LockRules::load(vault)?;

    let mut found = Vec::new();
    let mut omitted = 0usize;
    resolve::walk(
        vault,
        &RelPath::root(),
        1,
        max_depth,
        &mut found,
        &mut omitted,
    );

    // The same total order a folder `read` uses, and the reason two runs over
    // an unchanged vault produce byte-identical output.
    found.sort_by(|a, b| a.path.as_str().cmp(b.path.as_str()));

    let index = Index::build(found.iter().map(|note| &note.path));
    let mut nodes: Vec<GraphNode> = found
        .iter()
        .map(|note| GraphNode {
            path: note.path.to_string(),
            title: title_of(&note.path),
            locked: rules.is_locked(&note.path),
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


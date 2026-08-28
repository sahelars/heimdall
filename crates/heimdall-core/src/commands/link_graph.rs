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

use crate::commands::links::{self, LinkStyle};
use crate::errors::Result;
use crate::limits;
use crate::paths::{self, RelPath};
use crate::storage::Vault;
use crate::timestamps;

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
    walk(
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

    let index = Index::build(&found);
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

/// One Markdown file found by the walk.
struct Note {
    path: RelPath,
    size_bytes: u64,
    modified_at: String,
}

/// Collect every listable Markdown file, stopping at the node cap but still
/// counting what it had to leave out.
///
/// `list_documents`' walker is not reusable here: every one of its invariants —
/// the cursor, cursor-bounded subtree pruning, the page limit, the `aios/` skip
/// — is about pagination, and this needs the inverse of four of the five.
fn walk(
    vault: &Vault,
    directory: &RelPath,
    level: usize,
    max_depth: usize,
    include_aios: bool,
    found: &mut Vec<Note>,
    omitted: &mut usize,
) {
    let Ok(children) = vault.children(directory) else {
        return;
    };

    for child in children {
        if !paths::is_listable(&child.name) {
            continue;
        }
        let path = directory.join(&child.name);
        if !include_aios && path.is_in_aios() {
            continue;
        }

        if child.is_dir {
            if level < max_depth {
                walk(
                    vault,
                    &path,
                    level + 1,
                    max_depth,
                    include_aios,
                    found,
                    omitted,
                );
            }
            continue;
        }

        if !path.is_markdown() {
            continue;
        }
        if found.len() >= limits::GRAPH_MAX_NODES {
            // Counting past the cap costs one `read_dir` per directory and no
            // extra `stat`, so the omitted total is exact and nearly free.
            *omitted += 1;
            continue;
        }
        found.push(Note {
            path,
            size_bytes: child.meta.size_bytes,
            modified_at: timestamps::to_rfc3339(child.meta.modified_at),
        });
    }
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

/// Everything needed to turn a written target into a node index.
struct Index {
    /// Lowercased full path to node index.
    by_path: BTreeMap<String, usize>,
    /// Lowercased filename to every node with that name, in path order.
    by_name: BTreeMap<String, Vec<usize>>,
    /// The exact-case path of each node, for the case-sensitive tie-break.
    paths: Vec<String>,
}

impl Index {
    fn build(notes: &[Note]) -> Self {
        let mut by_path = BTreeMap::new();
        let mut by_name: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        let mut paths = Vec::with_capacity(notes.len());

        for (index, note) in notes.iter().enumerate() {
            let path = note.path.to_string();
            by_path.insert(path.to_ascii_lowercase(), index);
            if let Some(name) = note.path.file_name() {
                by_name
                    .entry(name.to_ascii_lowercase())
                    .or_default()
                    .push(index);
            }
            paths.push(path);
        }

        Self {
            by_path,
            by_name,
            paths,
        }
    }

    /// Resolve one written target against the vault, the way a vault editor does.
    fn resolve(&self, link: &links::RawLink, from: &RelPath) -> Option<usize> {
        let key = normalize(&link.target)?;

        // `..` is resolved here rather than left to `RelPath`, which refuses it.
        // An editor set to write relative paths produces `[x](../notes/a.md)`,
        // and dropping those would make a whole vault's Markdown links look
        // broken.
        let relative = walk_relative(&from.parent(), &key)?.to_ascii_lowercase();
        let absolute = key.to_ascii_lowercase();

        // The two styles disagree about which reading comes first, and it
        // matters whenever a vault has `target.md` at the root and another
        // beside the linking note. `[[target]]` is a vault-wide name, so the
        // vault-relative reading wins; `[x](target.md)` is a path written from
        // where the note sits, so the neighbour wins.
        let order = match link.style {
            LinkStyle::Wiki => [&absolute, &relative],
            LinkStyle::Markdown => [&relative, &absolute],
        };
        for candidate in order {
            if let Some(index) = self.by_path.get(candidate.as_str()) {
                return Some(*index);
            }
        }

        // Basename match — a wikilink names a note, a Markdown link names a
        // path. That distinction is the conventional one, which is why the style
        // is carried this far.
        if link.style != LinkStyle::Wiki {
            return None;
        }
        let name = key.rsplit('/').next()?.to_ascii_lowercase();
        let candidates = self.by_name.get(&name)?;
        match candidates.as_slice() {
            [] => None,
            [only] => Some(*only),
            many => Some(self.disambiguate(many, &key)),
        }
    }

    /// Pick one of several notes sharing a basename.
    ///
    /// Exact case first, then shortest-path-wins, then the shortest
    /// path in bytes, then the byte-wise smallest. That last step is not
    /// cosmetic: it makes the choice a total order, so two runs over an
    /// unchanged vault cannot disagree.
    fn disambiguate(&self, candidates: &[usize], key: &str) -> usize {
        let exact: Vec<usize> = candidates
            .iter()
            .copied()
            .filter(|index| self.paths[*index].ends_with(key))
            .collect();
        let pool = if exact.is_empty() { candidates } else { &exact };

        *pool
            .iter()
            .min_by(|a, b| {
                let (left, right) = (&self.paths[**a], &self.paths[**b]);
                left.matches('/')
                    .count()
                    .cmp(&right.matches('/').count())
                    .then_with(|| left.len().cmp(&right.len()))
                    .then_with(|| left.cmp(right))
            })
            .expect("a non-empty candidate list")
    }
}

/// Resolve a target written relative to `base`, honouring `..`.
///
/// Returns `None` when the path climbs above the vault root, which is not a
/// link to anywhere Heimdall can see.
fn walk_relative(base: &RelPath, key: &str) -> Option<String> {
    let mut parts: Vec<&str> = base.components().collect();
    for component in key.split('/') {
        match component {
            "." | "" => {}
            ".." => {
                parts.pop()?;
            }
            name => parts.push(name),
        }
    }
    Some(parts.join("/"))
}

/// Put a written target into the shape the path index uses.
fn normalize(target: &str) -> Option<String> {
    let target = target.replace('\\', "/");
    let target = target.trim().trim_start_matches("./").trim_end_matches('/');
    if target.is_empty() {
        return None;
    }
    // Only `.md` files are nodes, so a bare name means the Markdown file.
    let has_extension = target
        .rsplit('/')
        .next()
        .and_then(|name| name.rsplit_once('.'))
        .is_some_and(|(stem, ext)| !stem.is_empty() && ext.eq_ignore_ascii_case("md"));

    Some(if has_extension {
        target.to_string()
    } else {
        format!("{target}.md")
    })
}

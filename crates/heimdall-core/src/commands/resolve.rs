//! What notes exist, and where a written link points.
//!
//! Split out of `link_graph` when `relink` arrived, because the two must agree
//! exactly: the graph decides that `[[roadmap]]` in one note means
//! `projects/roadmap.md`, and the rewriter has to reach the same conclusion or
//! it will retarget a link the reader can see going somewhere else. One
//! implementation is the only way to guarantee that; two would drift the first
//! time either was touched.
//!
//! Pure resolution plus the walk that feeds it. No rewriting lives here.

use std::collections::BTreeMap;

use crate::commands::links::{self, LinkStyle};
use crate::limits;
use crate::paths::{self, RelPath};
use crate::storage::Vault;
use crate::timestamps;

/// One Markdown file found by the walk.
#[derive(Debug, Clone)]
pub(crate) struct Note {
    pub path: RelPath,
    pub size_bytes: u64,
    pub modified_at: String,
}

/// Collect every listable Markdown file, stopping at the node cap but still
/// counting what it had to leave out.
///
/// `list_documents`' walker is not reusable here: every one of its invariants —
/// the cursor, cursor-bounded subtree pruning, the page limit, the `aios/` skip
/// — is about pagination, and this needs the inverse of four of the five.
pub(crate) fn walk(
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

/// Everything needed to turn a written target into a node index.
pub(crate) struct Index {
    /// Lowercased full path to node index.
    by_path: BTreeMap<String, usize>,
    /// Lowercased filename to every node with that name, in path order.
    by_name: BTreeMap<String, Vec<usize>>,
    /// The exact-case path of each node, for the case-sensitive tie-break.
    paths: Vec<String>,
}

impl Index {
    /// Build over the paths the vault holds, in the order the caller intends
    /// their node indices to take.
    ///
    /// Paths rather than notes: `relink` resolves against a path set it has
    /// altered — the vault as it stood before a move — and has no file metadata
    /// to go with it.
    pub(crate) fn build<'a>(notes: impl IntoIterator<Item = &'a RelPath>) -> Self {
        let mut by_path = BTreeMap::new();
        let mut by_name: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        let mut paths = Vec::new();

        for (index, note) in notes.into_iter().enumerate() {
            let path = note.to_string();
            by_path.insert(path.to_ascii_lowercase(), index);
            if let Some(name) = note.file_name() {
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
    pub(crate) fn resolve(&self, link: &links::RawLink, from: &RelPath) -> Option<usize> {
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
pub(crate) fn walk_relative(base: &RelPath, key: &str) -> Option<String> {
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
pub(crate) fn normalize(target: &str) -> Option<String> {
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

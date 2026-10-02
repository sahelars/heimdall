//! `relink` — follow a move with the links that pointed at it (SPEC §15).
//!
//! A client operation, and deliberately a *separate* one from `move_path`. The
//! rule `move_path` states still holds: a move is one rename, not a multi-file
//! write. What changes is that the client no longer has to leave the breakage
//! behind — it calls this next, and this reports exactly what it changed.
//!
//! The objection the older design raised was to an *unbounded* write with *no
//! revision check*. Neither applies here:
//!
//! - **Bounded.** Only files holding a link to the moved path are written. The
//!   scan is the whole vault under the graph's own caps (§8) plus a substring
//!   prefilter, and the writes are capped again by `RELINK_MAX_FILES`.
//! - **Checked.** The whole read-modify-write runs inside one
//!   `with_write_lock` body, which is strictly stronger than the
//!   `expected_revision` a caller could supply: a revision check closes the gap
//!   between a client's read and its write, and here there is no gap to close.
//!   Every read and every write happens under the one lock, so no other
//!   Heimdall process can interleave. That is also why this calls
//!   `vault.read`/`vault.atomic_write` directly rather than `write` — a
//!   locking operation inside a lock body is a self-deadlock (SPEC §14).
//!
//! A locked note is never rewritten. Its links are left as they stand and the
//! note is named in `locked`, so the client can say which links a lock kept
//! broken rather than letting the user find them later.
//!
//! The interesting part is not the orchestration but deciding *which* `[[foo]]`
//! meant the note that moved, and what to write in its place. Both answers come
//! from `resolve`, the same module `link_graph` resolves with, so the rewriter
//! cannot disagree with the graph the user is looking at. A replacement is
//! proposed in the shape the link was written in and then **resolved again** to
//! confirm it lands on the moved note; only then is it spliced in. A link that
//! no proposal satisfies is reported and left exactly as it was.
//!
//! Deliberately no `JsonSchema` derive, for the reason `link_graph` gives:
//! a type without one cannot be given an MCP tool without a compile error.

use serde::{Deserialize, Serialize};

use crate::commands::links::{self, LinkStyle};
use crate::commands::resolve::{self, Index};
use crate::errors::{Error, Result};
use crate::limits;
use crate::notelocks::Locks;
use crate::paths::RelPath;
use crate::revisions::Revision;
use crate::storage::Vault;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RelinkRequest {
    /// Where the note or folder was, before the move.
    pub from: String,
    /// Where it is now.
    pub to: String,
    /// Report what would change without writing anything.
    #[serde(default)]
    pub dry_run: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct RelinkUpdate {
    pub path: String,
    /// How many links in this file were retargeted.
    pub links: usize,
    /// The revision the file now has — or would have, on a dry run.
    pub new_revision: Revision,
}

/// Why one link was left as it was written.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RelinkSkipReason {
    /// No way of writing the new target from this note resolves back to it.
    /// Rewriting anyway would point the link at a different note.
    Unresolvable,
}

#[derive(Debug, Clone, Serialize)]
pub struct RelinkSkip {
    pub path: String,
    /// The link's target, exactly as it stands in the file.
    pub target: String,
    pub reason: RelinkSkipReason,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct RelinkTruncation {
    /// The write budget ran out; `files_unscanned` counts what was not looked at.
    pub file_cap_hit: bool,
    pub node_cap_hit: bool,
    pub total_bytes_cap_hit: bool,
    pub nodes_omitted: usize,
    pub files_unscanned: usize,
    pub scanned_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct RelinkResponse {
    pub from: String,
    pub to: String,
    pub dry_run: bool,
    pub updated: Vec<RelinkUpdate>,
    pub skipped: Vec<RelinkSkip>,
    /// Notes that link at what moved but are locked, so were not rewritten.
    pub locked: Vec<String>,
    pub truncated: RelinkTruncation,
}

pub fn relink(vault: &Vault, request: RelinkRequest) -> Result<RelinkResponse> {
    let from = RelPath::parse(&request.from)?;
    let to = RelPath::parse(&request.to)?;
    if from.as_str() == to.as_str() {
        return Err(Error::invalid_input(
            "\"from\" and \"to\" are the same path; nothing moved",
        ));
    }
    let dry_run = request.dry_run;

    vault.with_write_lock(&to, || {
        let mut found = Vec::new();
        let mut nodes_omitted = 0usize;
        resolve::walk(
            vault,
            &RelPath::root(),
            1,
            limits::RECURSE_DEPTH_MAX,
            &mut found,
            &mut nodes_omitted,
        );
        found.sort_by(|a, b| a.path.as_str().cmp(b.path.as_str()));

        // The vault as it stood before the move: every path under `to` put back
        // where it came from. Resolving against this is what tells us a link
        // meant the note that moved, which it can no longer say for itself.
        let before: Vec<RelPath> = found
            .iter()
            .map(|note| unmove(&note.path, &from, &to))
            .collect();
        let moved: Vec<bool> = found
            .iter()
            .zip(&before)
            .map(|(note, was)| note.path.as_str() != was.as_str())
            .collect();

        let before_index = Index::build(before.iter());
        let after_index = Index::build(found.iter().map(|note| &note.path));

        let rules = Locks::load(vault)?;
        let mut updated = Vec::new();
        let mut skipped = Vec::new();
        let mut locked = Vec::new();
        let mut truncated = RelinkTruncation {
            node_cap_hit: nodes_omitted > 0,
            nodes_omitted,
            ..Default::default()
        };

        for (position, note) in found.iter().enumerate() {
            if updated.len() >= limits::RELINK_MAX_FILES {
                truncated.file_cap_hit = true;
                truncated.files_unscanned += found.len() - position;
                break;
            }
            if note.size_bytes > limits::GRAPH_MAX_FILE_BYTES {
                truncated.files_unscanned += 1;
                continue;
            }
            if truncated.scanned_bytes + note.size_bytes > limits::GRAPH_MAX_TOTAL_BYTES {
                truncated.total_bytes_cap_hit = true;
                truncated.files_unscanned += found.len() - position;
                break;
            }

            let Ok(bytes) = vault.read(&note.path) else {
                // One unreadable file must not fail a whole rename's follow-up.
                truncated.files_unscanned += 1;
                continue;
            };
            let Ok(text) = std::str::from_utf8(&bytes) else {
                truncated.files_unscanned += 1;
                continue;
            };
            truncated.scanned_bytes += note.size_bytes;

            // Most of a vault costs a substring search and no scan at all.
            if !text.contains("[[") && !text.contains("](") {
                continue;
            }

            let outcome = rewrite(
                text,
                &before[position],
                &note.path,
                moved[position],
                &Sides {
                    before: &before,
                    after: &found,
                    before_index: &before_index,
                    after_index: &after_index,
                    moved: &moved,
                },
            );

            if outcome.too_many_links {
                truncated.files_unscanned += 1;
                continue;
            }

            for target in outcome.unresolvable {
                skipped.push(RelinkSkip {
                    path: note.path.to_string(),
                    target,
                    reason: RelinkSkipReason::Unresolvable,
                });
            }

            let Some(rewritten) = outcome.text else {
                continue;
            };
            if rules.is_locked(&note.path) {
                locked.push(note.path.to_string());
                continue;
            }
            let new_revision = if dry_run {
                Revision::of_bytes(rewritten.as_bytes())
            } else {
                vault.atomic_write(&note.path, rewritten.as_bytes())?
            };
            updated.push(RelinkUpdate {
                path: note.path.to_string(),
                links: outcome.changed,
                new_revision,
            });
        }

        Ok(RelinkResponse {
            from: from.to_string(),
            to: to.to_string(),
            dry_run,
            updated,
            skipped,
            locked,
            truncated,
        })
    })
}

/// Where a path was before the move, given the move's endpoints.
///
/// A note move matches the path itself; a folder move matches everything under
/// it. Checking both covers the two without having to ask which happened.
fn unmove(path: &RelPath, from: &RelPath, to: &RelPath) -> RelPath {
    let path = path.as_str();
    if path == to.as_str() {
        return from.clone();
    }
    match path
        .strip_prefix(to.as_str())
        .and_then(|rest| rest.strip_prefix('/'))
    {
        Some(rest) => RelPath::parse(&format!("{from}/{rest}")).unwrap_or_else(|_| from.clone()),
        None => RelPath::parse(path).unwrap_or_else(|_| from.clone()),
    }
}

/// The vault on both sides of the move, which is what every rewrite is decided
/// against: where a link pointed before, and where the same text would point
/// now.
struct Sides<'a> {
    before: &'a [RelPath],
    after: &'a [resolve::Note],
    before_index: &'a Index,
    after_index: &'a Index,
    /// Which nodes the move touched, by the index both sides share.
    moved: &'a [bool],
}

struct Rewrite {
    /// The new text, or `None` when nothing in this file changed.
    text: Option<String>,
    changed: usize,
    unresolvable: Vec<String>,
    /// Past the per-file link cap, so the file was left alone entirely.
    too_many_links: bool,
}

/// Retarget every link in one file that pointed at something the move touched.
fn rewrite(text: &str, was: &RelPath, now: &RelPath, self_moved: bool, sides: &Sides) -> Rewrite {
    let mut edits: Vec<(std::ops::Range<usize>, String)> = Vec::new();
    let mut unresolvable = Vec::new();

    let links = links::scan(text);
    if links.len() > limits::GRAPH_MAX_LINKS_PER_FILE {
        // `link_graph` truncates a file like this and says so, because a partial
        // graph still draws. A partial rewrite does not: it would leave one note
        // with some links moved and some not, and the count reported to the user
        // would be of the ones it happened to reach. Leave it whole and say it
        // was not looked at.
        return Rewrite {
            text: None,
            changed: 0,
            unresolvable: Vec::new(),
            too_many_links: true,
        };
    }

    for link in links {
        // Where it pointed before the move. A file that itself moved has to be
        // resolved from where it used to sit, or its own relative links read
        // against the wrong folder.
        let Some(target) = sides.before_index.resolve(&link, was) else {
            continue;
        };
        // Either end of a link can be what moved. A note that changed folders
        // takes its own relative links with it, and `../sibling` read from the
        // new parent is a different note — or none.
        if !sides.moved[target] && !self_moved {
            continue;
        }
        let written = &text[link.span.clone()];

        match propose(
            text,
            &link,
            now,
            &sides.before[target],
            &sides.after[target].path,
            sides.after_index,
            target,
        ) {
            Some(replacement) if replacement != written => {
                edits.push((link.span.clone(), replacement))
            }
            // A bare `[[roadmap]]` under a folder rename still names the same
            // note. Nothing to write, and nothing went wrong.
            Some(_) => {}
            None => unresolvable.push(written.to_string()),
        }
    }

    if edits.is_empty() {
        return Rewrite {
            text: None,
            changed: 0,
            unresolvable,
            too_many_links: false,
        };
    }

    // Scan order is already file order, but splicing is unforgiving enough that
    // the invariant is worth asserting rather than assuming.
    edits.sort_by_key(|(span, _)| span.start);
    let mut rewritten = String::with_capacity(text.len());
    let mut cursor = 0usize;
    for (span, replacement) in &edits {
        debug_assert!(span.start >= cursor, "link spans overlap");
        if span.start < cursor {
            continue;
        }
        rewritten.push_str(&text[cursor..span.start]);
        rewritten.push_str(replacement);
        cursor = span.end;
    }
    rewritten.push_str(&text[cursor..]);

    Rewrite {
        text: Some(rewritten),
        changed: edits.len(),
        unresolvable,
        too_many_links: false,
    }
}

/// What to write in place of `written`, or `None` if nothing safe can be.
///
/// Every candidate is resolved again through the post-move index before it is
/// accepted. Preserving the shape a link was written in is a courtesy; landing
/// on the intended note is the requirement, and the second is what decides.
fn propose(
    text: &str,
    link: &links::RawLink,
    now: &RelPath,
    was_destination: &RelPath,
    destination: &RelPath,
    after_index: &Index,
    target: usize,
) -> Option<String> {
    let wiki = link.style == LinkStyle::Wiki;
    // Shape is read from the decoded target, not from the written bytes: a
    // Markdown destination arrives percent-escaped, and `plans/my%20note.md`
    // compared against a path would look like neither of the two shapes.
    let target_text = link.target.as_str();

    // A wikilink may drop the extension and usually does; a Markdown link is a
    // path and keeps it.
    let keep_extension = !wiki
        || target_text
            .rsplit('/')
            .next()
            .is_some_and(|name| name.to_ascii_lowercase().ends_with(".md"));

    // How the link named its target before the move, which is the shape worth
    // preserving. A bare wikilink is a name; anything else is a path, and the
    // only question is which end it was written from.
    let named_from_root = resolve::normalize(target_text)
        .is_some_and(|key| key.eq_ignore_ascii_case(was_destination.as_str()));

    // Ordered by how the link reads today, always ending at the unambiguous
    // vault-relative path — the one form nothing else can shadow.
    let mut candidates = Vec::new();
    if wiki && !target_text.contains('/') {
        candidates.push(trim_extension(
            destination.file_name().unwrap_or_default(),
            keep_extension,
        ));
    } else if !named_from_root {
        candidates.push(trim_extension(
            &relative_from(&now.parent(), destination),
            keep_extension,
        ));
    }
    candidates.push(trim_extension(destination.as_str(), keep_extension));

    // `[x](<my note.md>)`: the span sits inside the brackets, so they survive
    // the splice and the replacement must not escape what they already cover.
    let angled = link.span.start > 0 && text.as_bytes()[link.span.start - 1] == b'<';

    for candidate in candidates {
        let probe = links::RawLink {
            target: candidate.clone(),
            style: link.style,
            span: 0..0,
        };
        if after_index.resolve(&probe, now) == Some(target) {
            return Some(if wiki {
                candidate
            } else {
                encode_markdown(&candidate, angled)
            });
        }
    }
    None
}

/// Drop a trailing `.md` when the link did not carry one.
fn trim_extension(path: &str, keep: bool) -> String {
    if keep {
        return path.to_string();
    }
    path.strip_suffix(".md")
        .or_else(|| path.strip_suffix(".MD"))
        .unwrap_or(path)
        .to_string()
}

/// `target` written from inside `base`, using `..` to climb.
fn relative_from(base: &RelPath, target: &RelPath) -> String {
    let base: Vec<&str> = base.components().collect();
    let target: Vec<&str> = target.components().collect();
    let common = base
        .iter()
        .zip(&target)
        .take_while(|(left, right)| left == right)
        .count();

    let mut parts: Vec<&str> = vec![".."; base.len() - common];
    parts.extend_from_slice(&target[common..]);
    parts.join("/")
}

/// Escape what a Markdown destination cannot hold literally.
///
/// Skipped inside `<...>`, which is the other way of saying the same thing and
/// is already around the span being replaced.
fn encode_markdown(target: &str, angled: bool) -> String {
    if angled {
        return target.to_string();
    }
    target
        .chars()
        .map(|character| match character {
            ' ' => "%20".to_string(),
            '(' => "%28".to_string(),
            ')' => "%29".to_string(),
            '<' => "%3C".to_string(),
            '>' => "%3E".to_string(),
            other => other.to_string(),
        })
        .collect()
}

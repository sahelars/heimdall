//! `delete_path` — move one note or folder into the vault's `.trash/` (SPEC §15).
//!
//! A client operation; no `JsonSchema` derive, so it cannot become an MCP tool.
//!
//! Nothing is ever unlinked. Deletion is quiet — which is only acceptable if it
//! is also recoverable. `.trash/` at the vault root is the conventional folder
//! for a vault's local trash, and the leading dot means `paths::is_listable`
//! already keeps
//! it out of every listing, so a trashed note leaves the UI and discovery
//! without any special case.
//!
//! The trash mirrors the source layout rather than flattening it: `projects/a.md`
//! becomes `.trash/projects/a.md`. Mirroring is the only layout in which
//! restoring a file is unambiguous.
//!
//! Emptying the trash is deliberately not implemented. It is a destructive
//! operation with no undo, the user can do it in Finder, and there is no reason
//! for an application that refuses to unlink files to grow a command that does.
//!
//! Nothing locked is deleted — not the path, anything inside it, or anything
//! directly inside a locked folder.

use serde::{Deserialize, Serialize};

use crate::commands::types::DocumentKind;
use crate::errors::{Error, ErrorCode, Result};
use crate::notelocks::LockRules;
use crate::paths::{self, RelPath};
use crate::revisions::Revision;
use crate::storage::Vault;

/// How many numeric suffixes to try before giving up.
const MAX_COLLISION_SUFFIX: u32 = 99;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeletePathRequest {
    pub path: String,
    /// Optional guard for a file that changed since it was read.
    #[serde(default)]
    pub expected_revision: Option<Revision>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DeletePathResponse {
    pub path: String,
    /// Where it now lives, so the client can offer an undo.
    pub trashed_to: String,
    pub kind: DocumentKind,
}

pub fn delete_path(vault: &Vault, request: DeletePathRequest) -> Result<DeletePathResponse> {
    let path = RelPath::parse_file(&request.path)?;
    // Refusing hidden paths also means `.trash/` content cannot be deleted a
    // second time: emptying the trash is not this command's job.
    path.deny_hidden()?;

    let is_dir = vault.is_dir(&path);
    if !is_dir && !vault.is_file(&path) {
        return Err(Error::not_found(format!("\"{path}\" does not exist"))
            .with_detail("path", path.as_str()));
    }

    if let Some(expected) = &request.expected_revision {
        if is_dir {
            return Err(Error::invalid_input(
                "expected_revision applies to a file; a folder has no revision",
            )
            .with_detail("parameter", "expected_revision"));
        }
        let current = Revision::of_bytes(&vault.read(&path)?);
        if expected != &current {
            return Err(Error::revision_conflict(format!(
                "\"{path}\" changed since it was read; re-read it before deleting it"
            ))
            .with_detail("path", path.as_str())
            .with_detail("current_revision", current.as_str()));
        }
    }

    let trash = RelPath::parse(paths::TRASH_DIR).expect("the trash constant is a valid path");
    let destination_parent = join_all(&trash, &path.parent());
    vault.create_dir_all(&destination_parent)?;

    let name = path
        .file_name()
        .ok_or_else(|| Error::internal("delete target has no filename"))?;
    let (stem, extension) = split_extension(name);

    for suffix in 0..=MAX_COLLISION_SUFFIX {
        let candidate_name = match (suffix, extension) {
            (0, _) => name.to_string(),
            (n, Some(ext)) => format!("{stem}_{n:02}.{ext}"),
            (n, None) => format!("{stem}_{n:02}"),
        };
        let candidate = destination_parent.join(&candidate_name);

        // The write lock makes "not taken" and "take it" one decision, so two
        // deletes of same-named notes cannot pick the same trash slot.
        let outcome = vault.with_write_lock(&candidate, || {
            let mut rules = LockRules::load(vault)?;
            rules.deny_subtree(&path)?;
            rules.deny_change_in(&path.parent(), &path)?;

            vault.rename_no_replace(&path, &candidate)?;
            let before = rules.clone();
            rules.drop_within(&path);
            if rules != before {
                rules.save(vault)?;
            }
            Ok(())
        });
        match outcome {
            Ok(()) => {
                return Ok(DeletePathResponse {
                    path: path.to_string(),
                    trashed_to: candidate.to_string(),
                    kind: if is_dir {
                        DocumentKind::Directory
                    } else {
                        DocumentKind::Document
                    },
                })
            }
            Err(err) if err.code == ErrorCode::AlreadyExists => continue,
            Err(err) => return Err(err),
        }
    }

    Err(Error::already_exists(format!(
        "more than {MAX_COLLISION_SUFFIX} copies of \"{name}\" are already in the trash"
    ))
    .with_detail("path", path.as_str()))
}

/// Append every component of `tail` to `base`.
fn join_all(base: &RelPath, tail: &RelPath) -> RelPath {
    let mut joined = base.clone();
    for component in tail.components() {
        joined = joined.join(component);
    }
    joined
}

/// Split a filename into its stem and extension, so a suffix lands before the
/// dot: `note.md` collides as `note_01.md`, not `note.md_01`.
fn split_extension(name: &str) -> (&str, Option<&str>) {
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => (stem, Some(ext)),
        _ => (name, None),
    }
}

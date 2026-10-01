//! `lock` and `unlock` — make part of a vault read-only, or writable again
//! (SPEC §10).
//!
//! The target is the vault root, a folder, or a note. Locking a folder locks
//! everything in it, and so does unlocking — including notes that had been
//! given their own rule — after which a single note inside can be set apart
//! again. The rules themselves are in [`crate::notelocks`].
//!
//! A lock is enforced twice. Heimdall's own writers check the rules; and on
//! macOS every locked note also carries the filesystem's immutable flag, so a
//! shell, an editor, or an agent's file tools are refused by the operating
//! system too. Only `unlock` ever lifts that flag.
//!
//! Unlocking widens what an agent can change, so it needs a person: every
//! unlock passes a [`Presence`] check — Touch ID or the login password on a
//! Mac — before anything is touched. Locking narrows, and needs no one.
//!
//! Shell-only: a lock is the user's decision, so neither type derives
//! `JsonSchema` and neither can be given an MCP tool (SPEC §9).

use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::commands::types::DocumentKind;
use crate::errors::{Error, Result};
use crate::notelocks::{LockRules, Locks};
use crate::paths::RelPath;
use crate::storage::{Vault, FILE_PROTECTION};

/// How long one `lock` or `unlock` may spend flagging notes.
///
/// The sweep runs inside the vault's write lock, which other writers wait at
/// most ten seconds for. A vault too large to finish in this time is left
/// `partial`; running the same command again carries on from where it
/// stopped, because notes already in the right state cost one `stat`.
const SWEEP_LIMIT: Duration = Duration::from_secs(5);

/// Proof that a person, not a process acting for them, asked for something.
///
/// Implemented by the adapter: the shell asks with Touch ID or the login
/// password. Core only insists that it is asked — `unlock` cannot be called
/// without one — and that it is asked before the vault's write lock is taken,
/// so a prompt left on screen never stalls every other writer.
pub trait Presence {
    /// Ask the person at the computer to confirm `action`, a short phrase such
    /// as `unlock "projects/plan.md" in "Work"`. `Err` with `NOT_CONFIRMED`
    /// when they decline, cannot be asked, or do not answer.
    fn confirm(&self, action: &str) -> Result<()>;
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LockRequest {
    /// A folder or a Markdown note, relative to the vault root. Omit it for the
    /// whole vault.
    pub path: Option<String>,
}

/// How far the filesystem flags caught up with the rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Protection {
    /// Every note the command covers is flagged (or unflagged) to match.
    Complete,
    /// Some notes are not yet; `pending` says how many. Run it again.
    Partial,
    /// This platform has no flag Heimdall can set; the lock binds Heimdall's
    /// own writers only.
    Unsupported,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockResponse {
    /// The vault-relative path, as it is spelled on disk; empty for the whole
    /// vault.
    pub path: String,
    pub kind: DocumentKind,
    /// Whether the path is locked now.
    pub locked: bool,
    /// False when the path was already in this state and nothing was stored.
    pub changed: bool,
    /// Whether the notes it covers are protected at the filesystem level too.
    pub protection: Protection,
    /// Notes left to flag or unflag when `protection` is `partial`.
    pub pending: usize,
}

pub fn lock(vault: &Vault, request: LockRequest) -> Result<LockResponse> {
    let (path, kind) = target(vault, &request)?;
    apply(vault, path, kind, true)
}

/// Unlock `request.path`, once `presence` confirms a person asked for it.
pub fn unlock(vault: &Vault, request: LockRequest, presence: &dyn Presence) -> Result<LockResponse> {
    let (path, kind) = target(vault, &request)?;
    let what = if path.is_root() {
        "unlock the whole vault".to_string()
    } else {
        format!("unlock \"{path}\"")
    };
    presence.confirm(&format!("{what} in \"{}\"", vault.name()))?;
    apply(vault, path, kind, false)
}

/// Validate the target and spell it as it is stored.
fn target(vault: &Vault, request: &LockRequest) -> Result<(RelPath, DocumentKind)> {
    let typed = match &request.path {
        Some(raw) => RelPath::parse(raw)?,
        None => RelPath::root(),
    };
    typed.deny_hidden()?;

    if !typed.is_root() && !vault.entry_exists(&typed) {
        return Err(Error::not_found(format!("\"{typed}\" does not exist"))
            .with_detail("path", typed.as_str()));
    }
    // Rules are stored under the real spelling, so `Projects` and `projects`
    // are one rule and an alias cannot hold a second one.
    let path = vault.real_path(&typed)?;
    path.deny_hidden()?;

    let kind = if vault.is_dir(&path) {
        DocumentKind::Directory
    } else if vault.is_file(&path) && path.is_markdown() {
        DocumentKind::Document
    } else {
        return Err(Error::invalid_input(format!(
            "\"{typed}\" is not a folder or a Markdown note"
        ))
        .with_detail("path", typed.as_str()));
    };
    Ok((path, kind))
}

fn apply(vault: &Vault, path: RelPath, kind: DocumentKind, locked: bool) -> Result<LockResponse> {
    let (changed, protection, pending) = vault.with_write_lock(&path, || {
        let mut locks = Locks::load(vault)?;
        let loaded = locks.rules().clone();
        let rules = locks.rules_mut();
        rules.prune(vault);
        let changed = rules.set(&path, locked);
        if *locks.rules() != loaded {
            locks.save()?;
        }
        let (protection, pending) = sweep(vault, locks.rules(), &path, kind, locked);
        Ok((changed, protection, pending))
    })?;

    Ok(LockResponse {
        path: path.to_string(),
        kind,
        locked,
        changed,
        protection,
        pending,
    })
}

/// Bring the immutable flag on every note the command covers into line with
/// the rules, within [`SWEEP_LIMIT`].
///
/// Locking only ever adds flags and unlocking only ever lifts them: a flag is
/// never removed on the way to making something *more* locked, so nothing a
/// person locked by other means is undone by a `lock`.
fn sweep(
    vault: &Vault,
    rules: &LockRules,
    scope: &RelPath,
    kind: DocumentKind,
    locking: bool,
) -> (Protection, usize) {
    if !FILE_PROTECTION {
        return (Protection::Unsupported, 0);
    }
    let deadline = Instant::now() + SWEEP_LIMIT;
    let notes = match kind {
        DocumentKind::Document => vec![scope.clone()],
        DocumentKind::Directory => vault.notes_beneath(scope),
    };

    let mut pending = 0usize;
    for (done, note) in notes.iter().enumerate() {
        if Instant::now() >= deadline {
            pending = notes.len() - done;
            break;
        }
        let wanted = rules.is_locked(note);
        if wanted != locking {
            continue;
        }
        if vault.set_immutable(note, wanted).is_err() {
            pending += 1;
        }
    }

    if pending == 0 {
        (Protection::Complete, 0)
    } else {
        (Protection::Partial, pending)
    }
}

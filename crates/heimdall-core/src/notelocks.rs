//! Note locks: the read-only guardrail on notes and folders (SPEC §6, §10).
//!
//! Named apart from the write lock in [`crate::storage`], which is a mutex
//! serialising Heimdall's own writers. A note lock is something a person (or,
//! when asked, an agent) sets on part of a vault: a locked note can be read by
//! anyone and written by no one, the desktop editor included, until it is
//! unlocked.
//!
//! The state is a set of rules, each a vault path marked locked or unlocked. A
//! path's effective state is its **nearest rule**, looking at the path itself
//! and then each folder above it; with no rule anywhere it is unlocked. So:
//!
//! - locking the vault root locks everything;
//! - locking a folder locks everything in it, at every depth;
//! - a note can still be unlocked inside a locked folder, and locked inside an
//!   unlocked one — the more specific rule wins.
//!
//! Setting a rule on a folder clears every rule beneath it. "Lock this folder"
//! means every note in it, not every note but the ones someone once unlocked;
//! leaving the old exceptions in place would make a folder lock quietly partial.
//!
//! Rules live outside the vault, in the application-data directory beside the
//! write lock ([`Vault::state_path`]). Every change, and every check a write
//! makes against them, happens under the vault's write lock, so an unlock
//! cannot slip between a write's check and its rename.
//!
//! [`LockRules`] is the rules alone, as pure data. Every check an operation
//! makes goes through [`Locks`], which holds the rules *and* the vault, because
//! a rule is about a file and a caller names a file with a string: the check
//! is made against the spelling the caller used, the file's real on-disk
//! spelling ([`Vault::real_path`]), and the immutable flag a lock leaves on the
//! file itself. Any one of the three locking it is enough.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::appdata;
use crate::errors::{Error, Result};
use crate::paths::RelPath;
use crate::storage::Vault;

/// The on-disk shape. `root` is recorded so the file can be traced back to its
/// vault by a person looking in the data directory; it is never trusted.
#[derive(Debug, Default, Serialize, Deserialize)]
struct Stored {
    #[serde(default)]
    root: String,
    #[serde(default)]
    rules: BTreeMap<String, bool>,
}

/// One vault's lock rules, loaded.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LockRules {
    rules: BTreeMap<RelPath, bool>,
}

impl LockRules {
    /// Load a vault's rules. A vault nobody has locked anything in has no file.
    ///
    /// Each rule is re-keyed to its path's real spelling, so a rule stored as
    /// `Projects` — before rules were stored that way — still governs the
    /// `projects` folder it was set on. Where two rules land on one path, the
    /// locked one wins.
    pub fn load(vault: &Vault) -> Result<Self> {
        let stored: Option<Stored> = appdata::read_json(&vault.state_path())?;
        let mut rules: BTreeMap<RelPath, bool> = BTreeMap::new();
        for (raw, locked) in stored.map(|s| s.rules).unwrap_or_default() {
            let path = match RelPath::parse(&raw) {
                Ok(path) => vault.real_path(&path).unwrap_or(path),
                // A rule that no longer parses was edited by hand. An unlock
                // that cannot be read widens nothing by being ignored; a lock
                // that cannot be read must still lock something, and the only
                // thing it can safely mean is everything.
                Err(_) if locked => RelPath::root(),
                Err(_) => continue,
            };
            let entry = rules.entry(path).or_insert(locked);
            *entry |= locked;
        }
        Ok(Self { rules })
    }

    /// Persist the rules. Callers hold the vault's write lock.
    pub fn save(&self, vault: &Vault) -> Result<()> {
        let stored = Stored {
            root: vault.root().to_string(),
            rules: self
                .rules
                .iter()
                .map(|(path, locked)| (path.to_string(), *locked))
                .collect(),
        };
        appdata::write_json(&vault.state_path(), &stored)
    }

    /// The rule that locks `path`, or `None` when it is writable.
    pub fn locked_at(&self, path: &RelPath) -> Option<RelPath> {
        for candidate in path.ancestors() {
            if let Some(&locked) = self.rules.get(&candidate) {
                return locked.then_some(candidate);
            }
        }
        None
    }

    pub fn is_locked(&self, path: &RelPath) -> bool {
        self.locked_at(path).is_some()
    }

    /// The first locked thing at or beneath `path`: the path itself if it is
    /// locked, or else any locked rule inside it. A folder can be unlocked while
    /// a note inside it is not, and moving or deleting the folder moves or
    /// deletes that note too.
    pub fn locked_within(&self, path: &RelPath) -> Option<RelPath> {
        if let Some(at) = self.locked_at(path) {
            return Some(at);
        }
        self.rules
            .iter()
            .find(|(rule, &locked)| locked && rule.is_within(path))
            .map(|(rule, _)| rule.clone())
    }

    /// Lock or unlock `path` and everything beneath it. Returns whether the
    /// stored rules changed.
    pub fn set(&mut self, path: &RelPath, locked: bool) -> bool {
        let before = self.rules.clone();

        self.rules.retain(|rule, _| !rule.is_within(path));
        // A rule that says what the folder above already says is noise, and
        // noise that would outlive the folder rule it restates.
        let inherited = if path.is_root() {
            false
        } else {
            self.is_locked(&path.parent())
        };
        if inherited != locked {
            self.rules.insert(path.clone(), locked);
        }

        self.rules != before
    }

    /// Carry every rule at or beneath `from` to the same place beneath `to`,
    /// after a move. A locked note that moved stays locked.
    pub fn carry(&mut self, from: &RelPath, to: &RelPath) {
        let moved: Vec<(RelPath, bool)> = self
            .rules
            .iter()
            .filter_map(|(rule, &locked)| rule.rebase(from, to).map(|new| (new, locked)))
            .collect();
        self.rules.retain(|rule, _| !rule.is_within(from));
        self.rules.extend(moved);
    }

    /// Forget every rule at or beneath `path`, after it went to the trash. A
    /// note restored from the trash comes back writable, like any new note.
    pub fn drop_within(&mut self, path: &RelPath) {
        self.rules.retain(|rule, _| !rule.is_within(path));
    }

    /// Forget rules for paths that no longer exist — renamed or deleted by
    /// another editor, which Heimdall cannot see happen.
    pub fn prune(&mut self, vault: &Vault) {
        self.rules
            .retain(|rule, _| rule.is_root() || vault.entry_exists(rule));
    }

    /// Refuse a write to `path` when it is locked.
    pub fn deny_write(&self, path: &RelPath) -> Result<()> {
        match self.locked_at(path) {
            Some(at) => Err(locked_error(path, &at, format!("\"{path}\" is locked"))),
            None => Ok(()),
        }
    }

    /// Refuse to add or remove anything directly inside `folder` when it is
    /// locked. A locked folder's contents are fixed, not only their bytes.
    pub fn deny_change_in(&self, folder: &RelPath, path: &RelPath) -> Result<()> {
        match self.locked_at(folder) {
            Some(at) => {
                let name = if folder.is_root() {
                    "the vault".to_string()
                } else {
                    format!("folder \"{folder}\"")
                };
                Err(locked_error(path, &at, format!("{name} is locked")))
            }
            None => Ok(()),
        }
    }

    /// Refuse to move or delete `path` when it, or anything inside it, is locked.
    pub fn deny_subtree(&self, path: &RelPath) -> Result<()> {
        match self.locked_within(path) {
            Some(at) if at.is_within(path) && at != *path => Err(locked_error(
                path,
                &at,
                format!("\"{at}\" inside \"{path}\" is locked"),
            )),
            Some(at) => Err(locked_error(path, &at, format!("\"{path}\" is locked"))),
            None => Ok(()),
        }
    }
}

/// A vault's lock state as every check must see it: the rules, held against
/// the files they are about.
///
/// A path is locked when the nearest rule to the spelling the caller used says
/// so, when the nearest rule to its real on-disk spelling says so, or when the
/// file itself carries the immutable flag a lock sets. The last is what keeps
/// a lock standing if the rules are lost or the application-data directory is
/// redirected: the flag lives on the note, and Heimdall lifts it only in
/// `unlock`.
#[derive(Debug)]
pub struct Locks<'v> {
    vault: &'v Vault,
    rules: LockRules,
}

impl<'v> Locks<'v> {
    pub fn load(vault: &'v Vault) -> Result<Self> {
        Ok(Self {
            vault,
            rules: LockRules::load(vault)?,
        })
    }

    pub fn rules(&self) -> &LockRules {
        &self.rules
    }

    pub fn rules_mut(&mut self) -> &mut LockRules {
        &mut self.rules
    }

    /// What locks `path`: the rule responsible, or the file itself when it
    /// carries the immutable flag with no rule behind it. `None` when it is
    /// writable.
    pub fn locked_at(&self, path: &RelPath) -> Result<Option<RelPath>> {
        if let Some(at) = self.rules.locked_at(path) {
            return Ok(Some(at));
        }
        let real = self.vault.real_path(path)?;
        if real != *path {
            if let Some(at) = self.rules.locked_at(&real) {
                return Ok(Some(at));
            }
        }
        if self.vault.is_immutable(&real) {
            return Ok(Some(real));
        }
        Ok(None)
    }

    /// Whether `path` is locked. A path whose real spelling cannot be worked
    /// out is reported locked: this answers "may it be written", and not
    /// knowing is not "yes".
    pub fn is_locked(&self, path: &RelPath) -> bool {
        self.locked_at(path).map_or(true, |at| at.is_some())
    }

    /// Whether a rule locks `path`, under the spelling given or its real one.
    ///
    /// For reporting across many notes at once — a listing, the graph. It
    /// leaves out the per-file flag check, which costs an `open` per note: a
    /// flag with no rule behind it only exists when the rules were lost or a
    /// file was locked in Finder, and the note's own `read`, and every write,
    /// still see it.
    pub fn is_locked_by_rule(&self, path: &RelPath) -> bool {
        if self.rules.is_locked(path) {
            return true;
        }
        match self.vault.real_path(path) {
            Ok(real) => self.rules.is_locked(&real),
            Err(_) => true,
        }
    }

    /// [`Locks::is_locked_by_rule`] for `name` inside `dir`, given `dir`'s
    /// real spelling. A directory's entries are already spelled as they are
    /// stored, so resolving the directory once stands in for resolving every
    /// entry in it.
    pub fn is_locked_entry(&self, dir: &RelPath, real_dir: &RelPath, name: &str) -> bool {
        self.rules.is_locked(&dir.join(name)) || self.rules.is_locked(&real_dir.join(name))
    }

    /// Refuse a write to `path` when it is locked.
    pub fn deny_write(&self, path: &RelPath) -> Result<()> {
        match self.locked_at(path)? {
            Some(at) => Err(locked_error(path, &at, format!("\"{path}\" is locked"))),
            None => Ok(()),
        }
    }

    /// Refuse to add or remove anything directly inside `folder` when it is
    /// locked, under either spelling.
    pub fn deny_change_in(&self, folder: &RelPath, path: &RelPath) -> Result<()> {
        self.rules.deny_change_in(folder, path)?;
        let real = self.vault.real_path(folder)?;
        if real != *folder {
            self.rules.deny_change_in(&real, path)?;
        }
        Ok(())
    }

    /// Refuse to move or delete `path` when it, or anything inside it, is
    /// locked, under either spelling.
    pub fn deny_subtree(&self, path: &RelPath) -> Result<()> {
        self.rules.deny_subtree(path)?;
        let real = self.vault.real_path(path)?;
        if real != *path {
            self.rules.deny_subtree(&real)?;
        }
        if self.vault.is_immutable(&real) {
            return Err(locked_error(path, &real, format!("\"{path}\" is locked")));
        }
        Ok(())
    }

    /// The real spelling of `path`, which is the spelling rules are stored
    /// and carried under.
    pub fn real(&self, path: &RelPath) -> Result<RelPath> {
        self.vault.real_path(path)
    }

    pub fn save(&self) -> Result<()> {
        self.rules.save(self.vault)
    }
}

/// What a refusal says. Agents read this text, and it is the last thing one
/// sees before deciding what to do next, so it says plainly that the answer is
/// to stop — not to find another way to make the change.
fn locked_error(path: &RelPath, at: &RelPath, what: String) -> Error {
    let by = if at.is_root() {
        "the whole vault is locked".to_string()
    } else {
        format!("the lock is on \"{at}\"")
    };
    Error::locked(format!(
        "{what} ({by}). It is read-only by the user's decision: do not change it any other \
         way. Tell the user it is locked; only they can unlock it, in Heimdall"
    ))
    .with_detail("path", path.as_str())
    .with_detail("locked_at", at.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::ErrorCode;

    fn rel(raw: &str) -> RelPath {
        RelPath::parse(raw).unwrap()
    }

    #[test]
    fn nothing_is_locked_until_something_is() {
        let rules = LockRules::default();
        assert!(!rules.is_locked(&rel("a.md")));
        assert!(!rules.is_locked(&RelPath::root()));
    }

    #[test]
    fn a_folder_lock_reaches_every_depth_and_names_itself() {
        let mut rules = LockRules::default();
        rules.set(&rel("projects"), true);
        assert_eq!(rules.locked_at(&rel("projects/a/b.md")), Some(rel("projects")));
        assert!(!rules.is_locked(&rel("projects2/a.md")));
        assert!(!rules.is_locked(&rel("top.md")));
    }

    #[test]
    fn a_note_can_be_unlocked_inside_a_locked_folder() {
        let mut rules = LockRules::default();
        rules.set(&RelPath::root(), true);
        rules.set(&rel("drafts/open.md"), false);
        assert!(!rules.is_locked(&rel("drafts/open.md")));
        assert!(rules.is_locked(&rel("drafts/other.md")));
    }

    #[test]
    fn locking_a_folder_clears_the_exceptions_beneath_it() {
        let mut rules = LockRules::default();
        rules.set(&rel("projects"), true);
        rules.set(&rel("projects/open.md"), false);
        assert!(rules.set(&rel("projects"), true), "clearing an exception is a change");
        assert!(rules.is_locked(&rel("projects/open.md")));
    }

    #[test]
    fn restating_the_inherited_state_stores_nothing() {
        let mut rules = LockRules::default();
        assert!(!rules.set(&rel("a.md"), false));
        rules.set(&rel("projects"), true);
        assert!(!rules.set(&rel("projects/a.md"), true));
        assert_eq!(rules.rules.len(), 1);
    }

    #[test]
    fn a_locked_note_inside_an_unlocked_folder_still_pins_the_folder() {
        let mut rules = LockRules::default();
        rules.set(&rel("projects/keep.md"), true);
        assert!(!rules.is_locked(&rel("projects")));
        let err = rules.deny_subtree(&rel("projects")).unwrap_err();
        assert_eq!(err.code, ErrorCode::Locked);
        assert_eq!(err.details["locked_at"], "projects/keep.md");
    }

    #[test]
    fn rules_follow_a_move_and_leave_with_a_delete() {
        let mut rules = LockRules::default();
        rules.set(&rel("a/keep.md"), true);
        rules.carry(&rel("a"), &rel("b/a"));
        assert!(rules.is_locked(&rel("b/a/keep.md")));
        assert!(!rules.is_locked(&rel("a/keep.md")));

        rules.drop_within(&rel("b"));
        assert!(!rules.is_locked(&rel("b/a/keep.md")));
    }

    #[test]
    fn a_locked_folder_refuses_new_content_but_its_parent_does_not() {
        let mut rules = LockRules::default();
        rules.set(&rel("projects"), true);
        assert_eq!(
            rules.deny_change_in(&rel("projects"), &rel("projects/new.md")).unwrap_err().code,
            ErrorCode::Locked
        );
        rules.deny_change_in(&RelPath::root(), &rel("new.md")).unwrap();
    }
}

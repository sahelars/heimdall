//! Vault-relative path validation.
//!
//! Validation here is a fast pre-filter that rejects obviously hostile input
//! before it reaches the filesystem. The authority on containment is the
//! `cap-std` directory capability in [`crate::storage`] (SPEC §14) — never a
//! string-prefix check.

use std::fmt;

use crate::errors::{Error, Result};

/// Where `delete_path` moves removed content.
///
/// Deletion is recoverable rather than destructive: content is moved here, not
/// unlinked, so nothing a user removes is unrecoverable. The leading dot means
/// [`is_listable`] already keeps the folder out of every listing, so trashed
/// notes disappear from the UI and from discovery without a special case.
pub const TRASH_DIR: &str = ".trash";

/// Filesystem debris that is never part of a vault's content and is never
/// copied out of the embedded template.
const JUNK_NAMES: [&str; 3] = [".DS_Store", "Thumbs.db", "desktop.ini"];

/// A validated vault-relative path using `/` separators. The empty path is the
/// vault root.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RelPath(String);

impl RelPath {
    /// The vault root.
    pub fn root() -> Self {
        Self(String::new())
    }

    /// Validate caller-supplied path text.
    ///
    /// Rejects absolute paths, `..` traversal, NUL bytes, backslashes, Windows
    /// drive prefixes, and empty or dot components. Accepts `""`, `"."`, and
    /// `"/"` as the root.
    pub fn parse(raw: &str) -> Result<Self> {
        let trimmed = raw.trim_matches('/');
        if trimmed.is_empty() || trimmed == "." {
            return Ok(Self::root());
        }
        if raw.contains('\0') {
            return Err(invalid_path(raw, "contains a NUL byte"));
        }
        if raw.contains('\\') {
            return Err(invalid_path(raw, "must use \"/\" separators"));
        }

        let mut components = Vec::new();
        for component in trimmed.split('/') {
            if component.is_empty() {
                return Err(invalid_path(raw, "contains an empty path component"));
            }
            if component == "." || component == ".." {
                return Err(invalid_path(raw, "contains a \".\" or \"..\" component"));
            }
            if is_windows_prefix(component) {
                return Err(invalid_path(raw, "contains a drive or device prefix"));
            }
            components.push(component);
        }
        Ok(Self(components.join("/")))
    }

    /// Validate a path that must name a file, not the vault root.
    pub fn parse_file(raw: &str) -> Result<Self> {
        let path = Self::parse(raw)?;
        if path.is_root() {
            return Err(Error::invalid_input("a file path is required, not the vault root"));
        }
        Ok(path)
    }

    /// Validate a path that must name an existing Markdown document.
    pub fn parse_markdown(raw: &str) -> Result<Self> {
        let path = Self::parse_file(raw)?;
        if !path.is_markdown() {
            return Err(
                invalid_path(raw, "must be a Markdown file ending in \".md\"")
                    .with_detail("path", path.as_str()),
            );
        }
        Ok(path)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_root(&self) -> bool {
        self.0.is_empty()
    }

    pub fn components(&self) -> impl Iterator<Item = &str> {
        self.0.split('/').filter(|c| !c.is_empty())
    }

    pub fn file_name(&self) -> Option<&str> {
        self.0.rsplit('/').next().filter(|s| !s.is_empty())
    }

    pub fn is_markdown(&self) -> bool {
        self.file_name()
            .is_some_and(|name| name.rsplit_once('.').is_some_and(|(_, ext)| ext.eq_ignore_ascii_case("md")))
    }

    /// Append one already-validated component.
    pub fn join(&self, component: &str) -> Self {
        if self.is_root() {
            Self(component.to_string())
        } else {
            Self(format!("{}/{component}", self.0))
        }
    }

    /// The containing directory, or the root for a top-level entry.
    pub fn parent(&self) -> Self {
        match self.0.rsplit_once('/') {
            Some((parent, _)) => Self(parent.to_string()),
            None => Self::root(),
        }
    }

    /// Whether this path is `outer` itself or lies beneath it, compared by
    /// whole components: `projects2` is not inside `projects`.
    pub fn is_within(&self, outer: &RelPath) -> bool {
        let mut outer_components = outer.components();
        let mut own = self.components();
        loop {
            match (outer_components.next(), own.next()) {
                (None, _) => return true,
                (Some(_), None) => return false,
                (Some(a), Some(b)) if a == b => continue,
                (Some(_), Some(_)) => return false,
            }
        }
    }

    /// This path and every folder above it, nearest first, ending at the root.
    pub fn ancestors(&self) -> Vec<RelPath> {
        let mut chain = vec![self.clone()];
        let mut current = self.clone();
        while !current.is_root() {
            current = current.parent();
            chain.push(current.clone());
        }
        chain
    }

    /// Re-express a path at or beneath `from` as the same path beneath `to`,
    /// or `None` when it does not lie under `from`.
    pub fn rebase(&self, from: &RelPath, to: &RelPath) -> Option<RelPath> {
        if !self.is_within(from) {
            return None;
        }
        let mut rebased = to.clone();
        for component in self.components().skip(from.components().count()) {
            rebased = rebased.join(component);
        }
        Some(rebased)
    }

    /// Reject any path with a hidden component.
    ///
    /// [`RelPath::parse`] bars only `.` and `..`, so `.config/app.json` and
    /// `.trash/note.md` both parse. Reads are safe from them by accident,
    /// because every read operation also demands a `.md` extension; a write has
    /// no such shield, and writing into a hidden folder would corrupt whatever
    /// tool owns it, while writing into `.trash/` would create content no
    /// listing could ever show again.
    pub fn deny_hidden(&self) -> Result<()> {
        for component in self.components() {
            if is_hidden(component) {
                return Err(Error::invalid_input(format!(
                    "\"{self}\" is inside a hidden folder and is not vault content"
                ))
                .with_detail("path", self.as_str()));
            }
        }
        Ok(())
    }
}

impl fmt::Display for RelPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Validate the folder name for a new vault.
///
/// `heimdall create` takes one folder name, never an arbitrary path (SPEC §7);
/// the location comes from `--root`.
pub fn validate_vault_name(raw: &str) -> Result<&str> {
    let reject = |reason: &str| {
        Err(Error::invalid_input(format!("vault name {reason}")).with_detail("parameter", "name"))
    };

    if raw.is_empty() {
        return reject("must not be empty");
    }
    if raw.len() > 255 {
        return reject("is longer than 255 bytes");
    }
    if raw.contains('/') || raw.contains('\\') {
        return reject("must be a single folder name, not a path");
    }
    if raw.contains('\0') {
        return reject("contains a NUL byte");
    }
    if raw == "." || raw == ".." {
        return reject("must not be \".\" or \"..\"");
    }
    if raw.starts_with('.') {
        return reject("must not start with \".\"");
    }
    if is_windows_prefix(raw) {
        return reject("must not be a drive or device name");
    }
    if raw.trim() != raw {
        return reject("must not begin or end with whitespace");
    }
    Ok(raw)
}

/// Whether a directory entry name is hidden (dotfile) and so not vault content.
pub fn is_hidden(name: &str) -> bool {
    name.starts_with('.')
}

/// Whether a name is filesystem debris that should never be listed or copied.
pub fn is_junk(name: &str) -> bool {
    JUNK_NAMES.iter().any(|junk| junk.eq_ignore_ascii_case(name))
}

/// Whether a name is one of Heimdall's own sidecar files.
///
/// Write temps are current: an atomic write lands in a temporary sibling before
/// being renamed into place. The `.lock` half is legacy — the write lock lives
/// outside the vault now (SPEC §14) — but a vault used before that change still
/// has one beside every note it ever wrote, and those must stay invisible.
pub fn is_heimdall_internal(name: &str) -> bool {
    name.starts_with('.') && (name.ends_with(".lock") || name.contains(".tmp-"))
}

/// Whether a name should appear in any listing.
pub fn is_listable(name: &str) -> bool {
    !is_hidden(name) && !is_junk(name) && !is_heimdall_internal(name)
}

fn is_windows_prefix(component: &str) -> bool {
    let bytes = component.as_bytes();
    (bytes.len() == 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':')
        || component.starts_with("\\\\")
        || component.contains(':')
}

fn invalid_path(raw: &str, reason: &str) -> Error {
    let _ = raw;
    Error::invalid_input(format!("path {reason}")).with_detail("parameter", "path")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::ErrorCode;

    #[test]
    fn root_has_several_accepted_spellings() {
        for raw in ["", ".", "/", "//"] {
            assert!(RelPath::parse(raw).unwrap().is_root(), "{raw:?}");
        }
    }

    #[test]
    fn traversal_and_absolute_paths_are_rejected() {
        for raw in [
            "..",
            "../secrets.md",
            "projects/../../escape.md",
            "projects/./a.md",
            "projects//a.md",
            "a\\b.md",
            "C:/notes.md",
            "note\0.md",
        ] {
            let err = RelPath::parse(raw).unwrap_err();
            assert_eq!(err.code, ErrorCode::InvalidInput, "accepted {raw:?}");
        }
    }

    #[test]
    fn leading_slashes_are_stripped_rather_than_treated_as_absolute() {
        // cap-std would reject an absolute path outright; normalizing here keeps
        // the error message about the real problem when there is one.
        assert_eq!(RelPath::parse("/projects/a.md").unwrap().as_str(), "projects/a.md");
    }

    #[test]
    fn markdown_detection_is_extension_based_and_case_insensitive() {
        assert!(RelPath::parse("a.md").unwrap().is_markdown());
        assert!(RelPath::parse("a.MD").unwrap().is_markdown());
        assert!(!RelPath::parse("a.markdown").unwrap().is_markdown());
        assert!(!RelPath::parse("a").unwrap().is_markdown());
        assert!(RelPath::parse_markdown("notes.txt").is_err());
    }

    #[test]
    fn parent_and_join_stay_within_the_relative_space() {
        let path = RelPath::parse("projects/lens/profile.md").unwrap();
        assert_eq!(path.parent().as_str(), "projects/lens");
        assert_eq!(path.file_name(), Some("profile.md"));
        assert_eq!(RelPath::parse("a.md").unwrap().parent().as_str(), "");
        assert_eq!(RelPath::root().join("ideas").as_str(), "ideas");
        assert_eq!(RelPath::parse("ideas").unwrap().join("drafts").as_str(), "ideas/drafts");
    }

    #[test]
    fn containment_is_by_whole_components() {
        let projects = RelPath::parse("projects").unwrap();
        assert!(RelPath::parse("projects/a.md").unwrap().is_within(&projects));
        assert!(projects.is_within(&projects));
        assert!(!RelPath::parse("projects2/a.md").unwrap().is_within(&projects));
        assert!(RelPath::parse("anything").unwrap().is_within(&RelPath::root()));
    }

    #[test]
    fn ancestors_run_from_the_path_to_the_root() {
        let chain: Vec<_> = RelPath::parse("a/b/c.md").unwrap().ancestors();
        let chain: Vec<_> = chain.iter().map(RelPath::as_str).collect();
        assert_eq!(chain, ["a/b/c.md", "a/b", "a", ""]);
    }

    #[test]
    fn rebasing_moves_a_subtree_and_ignores_anything_outside_it() {
        let from = RelPath::parse("a").unwrap();
        let to = RelPath::parse("x/y").unwrap();
        assert_eq!(RelPath::parse("a/b.md").unwrap().rebase(&from, &to).unwrap().as_str(), "x/y/b.md");
        assert_eq!(from.rebase(&from, &to).unwrap().as_str(), "x/y");
        assert!(RelPath::parse("ab.md").unwrap().rebase(&from, &to).is_none());
    }

    #[test]
    fn listings_exclude_hidden_junk_and_sidecar_files() {
        assert!(is_listable("notes.md"));
        assert!(!is_listable(".config"));
        assert!(!is_listable(".DS_Store"));
        assert!(is_junk(".ds_store"));
        assert!(!is_listable(".memory.md.lock"));
        assert!(!is_listable(".memory.md.tmp-1234-0"));
    }
}

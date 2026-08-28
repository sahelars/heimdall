//! Vault-relative path validation and the fixed managed locations.
//!
//! Validation here is a fast pre-filter that rejects obviously hostile input
//! before it reaches the filesystem. The authority on containment is the
//! `cap-std` directory capability in [`crate::storage`] (SPEC §14) — never a
//! string-prefix check.

use std::fmt;

use crate::errors::{Error, Result};

/// The protected subtree. Ordinary document operations exclude it entirely.
pub const AIOS_DIR: &str = "aios";
pub const MEMORIES_DIR: &str = "aios/memories";
pub const MAIN_MEMORY_FILE: &str = "aios/memories/memory.md";
pub const EXTENDED_DIR: &str = "aios/memories/extended";
pub const CONVERSATIONS_DIR: &str = "aios/conversations";
pub const NOTIFICATIONS_DIR: &str = "aios/notifications";
pub const ATTACHMENTS_DIR: &str = "aios/attachments";

/// Where `delete_path` moves removed content.
///
/// Deletion is recoverable rather than destructive: content is moved here, not
/// unlinked, so nothing a user removes is unrecoverable. The leading dot means
/// [`is_listable`] already keeps the folder out of every listing, so trashed
/// notes disappear from the UI and from discovery without a special case.
pub const TRASH_DIR: &str = ".trash";

/// Managed directories that must exist for a vault to count as initialized.
pub const REQUIRED_DIRS: [&str; 6] = [
    AIOS_DIR,
    MEMORIES_DIR,
    EXTENDED_DIR,
    CONVERSATIONS_DIR,
    NOTIFICATIONS_DIR,
    ATTACHMENTS_DIR,
];

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

    /// Whether this path is inside the protected `aios/` tree.
    ///
    /// The comparison is case-insensitive: on macOS and Windows `AIOS/notes.md`
    /// and `aios/notes.md` are the same file, so a case-sensitive check would
    /// let ordinary document operations reach protected content.
    pub fn is_in_aios(&self) -> bool {
        self.components()
            .next()
            .is_some_and(|first| first.eq_ignore_ascii_case(AIOS_DIR))
    }

    /// Reject any path that would let an ordinary document operation touch
    /// protected content (SPEC §6).
    pub fn deny_aios(&self) -> Result<()> {
        if self.is_in_aios() {
            return Err(Error::not_found(format!(
                "\"{AIOS_DIR}/\" holds protected content and is not reachable through document operations"
            ))
            .with_detail("path", self.as_str()));
        }
        Ok(())
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

/// Whether a path is managed structure that [`Vault::ensure_initialized`] needs.
///
/// Moving or deleting one of these would leave a vault that no longer opens, so
/// the client operations refuse them. The content *inside* the protected tree —
/// entries, extended memories — stays removable; only the skeleton is fixed.
///
/// The comparison is case-insensitive for the reason SPEC §6 gives about
/// `aios/`: on macOS and Windows `AIOS/Memories` names the same directory.
///
/// [`Vault::ensure_initialized`]: crate::storage::Vault::ensure_initialized
pub fn is_structural(path: &RelPath) -> bool {
    let candidates = REQUIRED_DIRS.iter().chain(std::iter::once(&MAIN_MEMORY_FILE));

    candidates.into_iter().any(|managed| {
        let mut wanted = managed.split('/');
        let mut actual = path.components();
        loop {
            match (wanted.next(), actual.next()) {
                (None, None) => return true,
                (None, Some(_)) | (Some(_), None) => return false,
                (Some(a), Some(b)) if a.eq_ignore_ascii_case(b) => continue,
                (Some(_), Some(_)) => return false,
            }
        }
    })
}

impl fmt::Display for RelPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A validated single filename with no directory part.
///
/// Used wherever the contract accepts a name rather than a path: `extended` on
/// memory operations and `id` on entry operations (SPEC §10).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BaseName(String);

impl BaseName {
    /// Validate a Markdown filename supplied by a caller.
    pub fn parse_markdown(raw: &str, parameter: &str) -> Result<Self> {
        let reject = |reason: &str| {
            Err(Error::invalid_input(format!("{parameter} {reason}"))
                .with_detail("parameter", parameter))
        };

        if raw.is_empty() {
            return reject("must not be empty");
        }
        if raw.len() > 255 {
            return reject("is longer than 255 bytes");
        }
        if raw.contains('/') || raw.contains('\\') {
            return reject("must be a bare filename, not a path");
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
        if !raw.rsplit_once('.').is_some_and(|(stem, ext)| !stem.is_empty() && ext.eq_ignore_ascii_case("md")) {
            return reject("must be a Markdown filename ending in \".md\"");
        }
        Ok(Self(raw.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for BaseName {
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
    fn aios_detection_ignores_case() {
        for raw in ["aios/notes.md", "AIOS/notes.md", "AiOs/memories/memory.md"] {
            let path = RelPath::parse(raw).unwrap();
            assert!(path.is_in_aios(), "{raw:?} not detected as protected");
            assert_eq!(path.deny_aios().unwrap_err().code, ErrorCode::NotFound);
        }
        assert!(!RelPath::parse("aiosphere/note.md").unwrap().is_in_aios());
        assert!(!RelPath::parse("projects/aios/note.md").unwrap().is_in_aios());
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
        assert_eq!(RelPath::root().join("aios").as_str(), "aios");
        assert_eq!(RelPath::parse("aios").unwrap().join("memories").as_str(), "aios/memories");
    }

    #[test]
    fn base_names_reject_anything_that_is_not_a_bare_markdown_file() {
        assert_eq!(
            BaseName::parse_markdown("project_history.md", "extended").unwrap().as_str(),
            "project_history.md"
        );
        for raw in [
            "",
            "..",
            ".hidden.md",
            "extended/history.md",
            "..\\escape.md",
            "/absolute.md",
            "history.txt",
            ".md",
            "C:.md",
        ] {
            assert!(
                BaseName::parse_markdown(raw, "extended").is_err(),
                "accepted {raw:?}"
            );
        }
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

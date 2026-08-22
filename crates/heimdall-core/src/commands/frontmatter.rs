//! Reading the two Heimdall-owned scalars out of an entry's leading block.
//!
//! `create_entry` writes `created_at` and `type` and refuses caller frontmatter
//! outright, which is right for a create — there is nothing to preserve yet. An
//! *edit* needs the narrower rule: those two fields must survive untouched,
//! while every other key belongs to the user and passes through. So this parser
//! is deliberately incurious. It extracts two scalars and ignores everything
//! else, including the list-valued `links:` an Obsidian vault registers as
//! `multitext`.

/// The Heimdall-owned scalars of a leading frontmatter block.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Owned {
    pub created_at: Option<String>,
    pub kind: Option<String>,
}

impl Owned {
    /// Whether Heimdall wrote either field, i.e. whether there is anything to
    /// preserve across an edit.
    pub fn is_present(&self) -> bool {
        self.created_at.is_some() || self.kind.is_some()
    }
}

/// Parse the leading `---` block, or `None` when there is not one.
///
/// An unterminated block is not a block: a note whose body happens to open with
/// `---` as a horizontal rule must not be read as frontmatter.
pub(crate) fn owned(content: &str) -> Option<Owned> {
    let content = content.strip_prefix('\u{feff}').unwrap_or(content);
    let mut lines = content.split('\n');

    let first = lines.next()?;
    if first.trim_end_matches('\r') != "---" {
        return None;
    }

    let mut found = Owned::default();
    let mut terminated = false;

    for line in lines {
        let line = line.trim_end_matches('\r');
        if line == "---" {
            terminated = true;
            break;
        }

        // A continuation line — the items of `links:`, or a nested mapping — is
        // part of the value above it, never a key of its own.
        if line.starts_with(' ') || line.starts_with('\t') {
            continue;
        }

        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim().to_string();

        // First occurrence wins: a duplicated key is what a YAML parser would
        // reject, and taking the last one would let an edit append a second
        // `created_at` that silently overrode the real one.
        match key.trim() {
            "created_at" if found.created_at.is_none() => found.created_at = Some(value),
            "type" if found.kind.is_none() => found.kind = Some(value),
            _ => {}
        }
    }

    terminated.then_some(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_heimdall_entry_yields_both_owned_fields() {
        let parsed = owned("---\ncreated_at: 2026-08-16T14:30:00Z\ntype: conversation\n---\n\n# S\n")
            .expect("a terminated block");
        assert_eq!(parsed.created_at.as_deref(), Some("2026-08-16T14:30:00Z"));
        assert_eq!(parsed.kind.as_deref(), Some("conversation"));
        assert!(parsed.is_present());
    }

    #[test]
    fn keys_the_user_owns_are_neither_read_nor_disturbed() {
        let parsed = owned(
            "---\ncreated_at: 2026-08-16T14:30:00Z\ntype: notification\nlinks:\n  - \"[[profile]]\"\ntags: [a, b]\n---\nbody\n",
        )
        .expect("a terminated block");
        assert_eq!(parsed.created_at.as_deref(), Some("2026-08-16T14:30:00Z"));
        assert_eq!(parsed.kind.as_deref(), Some("notification"));
    }

    #[test]
    fn an_indented_list_item_is_not_mistaken_for_a_key() {
        // `  - type: x` is an item of the list above it, not a `type` field.
        let parsed = owned("---\nlinks:\n  - type: conversation\n---\n").expect("a block");
        assert_eq!(parsed.kind, None);
    }

    #[test]
    fn an_unterminated_block_is_not_frontmatter() {
        assert!(owned("---\ncreated_at: x\n\nstill going\n").is_none());
    }

    #[test]
    fn content_that_does_not_open_with_a_fence_has_no_frontmatter() {
        assert!(owned("# Just a note\n\n---\n").is_none());
    }

    #[test]
    fn a_hand_made_entry_parses_to_an_empty_but_present_block() {
        let parsed = owned("---\ntags: [x]\n---\nbody\n").expect("a terminated block");
        assert!(!parsed.is_present());
    }

    #[test]
    fn a_byte_order_mark_does_not_hide_the_block() {
        let parsed = owned("\u{feff}---\ntype: conversation\n---\n").expect("a block");
        assert_eq!(parsed.kind.as_deref(), Some("conversation"));
    }

    #[test]
    fn carriage_returns_do_not_hide_the_fences() {
        let parsed = owned("---\r\ntype: conversation\r\n---\r\nbody\r\n").expect("a block");
        assert_eq!(parsed.kind.as_deref(), Some("conversation"));
    }
}

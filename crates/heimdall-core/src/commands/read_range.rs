//! The bounded-read engine behind every read operation (SPEC §8).
//!
//! Content is split only at line boundaries, which are always UTF-8 boundaries,
//! so a returned range is never a fragment of a character. A partial read is
//! always labelled partial — nothing silently implies a complete file.

use crate::commands::types::ReadResult;
use crate::errors::{Error, Result};
use crate::paths::RelPath;
use crate::revisions::Revision;
use crate::storage::Vault;

/// Read one bounded range of one file.
///
/// `byte_budget` caps the returned content. A file whose next line alone
/// exceeds the budget yields `LIMIT_EXCEEDED` rather than a split character or
/// a silently dropped line.
pub(crate) fn read_range(
    vault: &Vault,
    path: &RelPath,
    start_line: usize,
    max_lines: usize,
    byte_budget: usize,
) -> Result<ReadResult> {
    let bytes = vault.read(path)?;
    let size_bytes = bytes.len() as u64;
    let revision = Revision::of_bytes(&bytes);

    let text = std::str::from_utf8(&bytes).map_err(|_| {
        Error::io_error(format!("\"{path}\" is not valid UTF-8 and cannot be read as Markdown"))
            .with_detail("path", path.as_str())
    })?;

    let lines = split_lines(text);
    let total_lines = lines.len();

    // Reading past the end is a legitimate continuation result, not an error:
    // a caller following `next_line` to the boundary lands here.
    if start_line > total_lines {
        return Ok(ReadResult {
            path: path.to_string(),
            content: String::new(),
            start_line,
            end_line: start_line.saturating_sub(1),
            next_line: None,
            complete: true,
            size_bytes,
            revision,
        });
    }

    let mut used = 0usize;
    let mut taken = 0usize;
    for line in lines.iter().skip(start_line - 1).take(max_lines) {
        if used + line.len() > byte_budget {
            break;
        }
        used += line.len();
        taken += 1;
    }

    if taken == 0 {
        let needed = lines[start_line - 1].len();
        return Err(Error::limit_exceeded(format!(
            "line {start_line} of \"{path}\" is {needed} bytes, above the {byte_budget} byte budget \
             for this request; raise max_total_bytes or read fewer files"
        ))
        .with_detail("path", path.as_str())
        .with_detail("line", start_line)
        .with_detail("required_bytes", needed)
        .with_detail("available_bytes", byte_budget));
    }

    let end_line = start_line + taken - 1;
    let complete = end_line == total_lines;
    let content: String = lines[start_line - 1..end_line].concat();

    Ok(ReadResult {
        path: path.to_string(),
        content,
        start_line,
        end_line,
        next_line: (!complete).then_some(end_line + 1),
        complete,
        size_bytes,
        revision,
    })
}

/// Split into lines that keep their terminators, so concatenating a range
/// reproduces the original bytes exactly.
fn split_lines(text: &str) -> Vec<&str> {
    let mut lines = Vec::new();
    let mut start = 0;
    for (index, byte) in text.bytes().enumerate() {
        if byte == b'\n' {
            lines.push(&text[start..=index]);
            start = index + 1;
        }
    }
    if start < text.len() {
        lines.push(&text[start..]);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::ErrorCode;
    use camino::Utf8PathBuf;

    fn vault_with(name: &str, content: &[u8]) -> (tempfile::TempDir, Vault, RelPath) {
        let dir = tempfile::tempdir().unwrap();
        let root = Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).unwrap();
        let vault = Vault::open(&root).unwrap();
        let path = RelPath::parse(name).unwrap();
        vault.atomic_write(&path, content).unwrap();
        (dir, vault, path)
    }

    #[test]
    fn lines_keep_their_terminators_so_ranges_rejoin_exactly() {
        assert_eq!(split_lines("a\nb\n"), ["a\n", "b\n"]);
        assert_eq!(split_lines("a\nb"), ["a\n", "b"]);
        assert_eq!(split_lines("\n"), ["\n"]);
        assert_eq!(split_lines(""), Vec::<&str>::new());
    }

    #[test]
    fn a_short_file_is_returned_complete() {
        let (_tmp, vault, path) = vault_with("a.md", b"# My Project\n");
        let result = read_range(&vault, &path, 1, 200, 65_536).unwrap();

        assert_eq!(result.content, "# My Project\n");
        assert_eq!((result.start_line, result.end_line), (1, 1));
        assert!(result.complete);
        assert_eq!(result.next_line, None);
        assert_eq!(result.size_bytes, 13);
    }

    #[test]
    fn a_truncated_read_reports_where_to_continue() {
        let (_tmp, vault, path) = vault_with("a.md", b"one\ntwo\nthree\nfour\n");
        let first = read_range(&vault, &path, 1, 2, 65_536).unwrap();

        assert_eq!(first.content, "one\ntwo\n");
        assert_eq!(first.end_line, 2);
        assert!(!first.complete);
        assert_eq!(first.next_line, Some(3));

        let second = read_range(&vault, &path, first.next_line.unwrap(), 2, 65_536).unwrap();
        assert_eq!(second.content, "three\nfour\n");
        assert!(second.complete);
        assert_eq!(first.content + &second.content, "one\ntwo\nthree\nfour\n");
    }

    #[test]
    fn reading_past_the_end_returns_empty_and_complete() {
        let (_tmp, vault, path) = vault_with("a.md", b"one\n");
        let result = read_range(&vault, &path, 5, 200, 65_536).unwrap();

        assert_eq!(result.content, "");
        assert!(result.complete);
        assert_eq!(result.next_line, None);
    }

    #[test]
    fn an_empty_file_reads_as_complete_and_empty() {
        let (_tmp, vault, path) = vault_with("a.md", b"");
        let result = read_range(&vault, &path, 1, 200, 65_536).unwrap();

        assert_eq!(result.content, "");
        assert_eq!(result.end_line, 0);
        assert!(result.complete);
        assert_eq!(result.size_bytes, 0);
    }

    #[test]
    fn the_byte_budget_stops_at_a_line_boundary() {
        let (_tmp, vault, path) = vault_with("a.md", b"aaaa\nbbbb\ncccc\n");
        // Room for two 5-byte lines but not the third.
        let result = read_range(&vault, &path, 1, 200, 12).unwrap();

        assert_eq!(result.content, "aaaa\nbbbb\n");
        assert_eq!(result.next_line, Some(3));
        assert!(!result.complete);
    }

    #[test]
    fn multibyte_characters_are_never_split() {
        // Four-byte characters straddle any naive byte cut.
        let (_tmp, vault, path) = vault_with("a.md", "🌍🌍\n🌍\n".as_bytes());
        let result = read_range(&vault, &path, 1, 200, 10).unwrap();

        assert_eq!(result.content, "🌍🌍\n");
        assert_eq!(result.next_line, Some(2));
    }

    #[test]
    fn a_line_larger_than_the_budget_is_reported_not_silently_dropped() {
        let (_tmp, vault, path) = vault_with("a.md", b"aaaaaaaaaaaaaaaaaaaa\n");
        let err = read_range(&vault, &path, 1, 200, 8).unwrap_err();

        assert_eq!(err.code, ErrorCode::LimitExceeded);
        assert_eq!(err.details["required_bytes"], 21);
    }

    #[test]
    fn invalid_utf8_is_an_io_error_rather_than_lossy_content() {
        let (_tmp, vault, path) = vault_with("a.md", &[0x23, 0x20, 0xff, 0xfe, 0x0a]);
        let err = read_range(&vault, &path, 1, 200, 65_536).unwrap_err();

        assert_eq!(err.code, ErrorCode::IoError);
        assert!(err.message.contains("not valid UTF-8"), "{}", err.message);
    }

    #[test]
    fn the_revision_covers_the_whole_file_not_the_returned_range() {
        let (_tmp, vault, path) = vault_with("a.md", b"one\ntwo\n");
        let partial = read_range(&vault, &path, 1, 1, 65_536).unwrap();
        let whole = read_range(&vault, &path, 1, 200, 65_536).unwrap();

        assert_eq!(partial.revision, whole.revision);
        assert_eq!(partial.revision, Revision::of_bytes(b"one\ntwo\n"));
    }
}

//! Finding the links in one Markdown file.
//!
//! Pure: text in, link targets out, no filesystem. Resolution happens in
//! `link_graph`, which is the only place that knows what notes exist.
//!
//! Two deliberate divergences from CommonMark, both because this reads real
//! notes rather than conformance fixtures:
//!
//! - **Four-space indentation is not a code block.** Indented code is
//!   indistinguishable from a nested list item, and nested lists full of links
//!   are ordinary in a vault. Honouring CommonMark here would silently drop real
//!   edges; a fenced block is the unambiguous signal and the one this honours.
//! - **Inline code spans are line-local.** CommonMark lets a span cross lines
//!   inside a paragraph. Tracking that costs a paragraph-level parser to
//!   suppress links almost nobody writes that way.
//!
//! Links inside HTML comments are not suppressed either. Worth knowing; not
//! worth an HTML parser.

use std::ops::Range;

/// How a link was written, which decides how it may be resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LinkStyle {
    /// `[[target]]` — a name resolved across the whole vault.
    Wiki,
    /// `[text](target)` — a path, resolved relative to the linking note.
    Markdown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RawLink {
    pub target: String,
    pub style: LinkStyle,
    /// Where the target sits in the text it was scanned from, in bytes.
    ///
    /// This is the range of the *written* target — before a Markdown link's
    /// percent escapes are decoded, and inside the delimiters rather than
    /// around them — so replacing exactly this range leaves the `|alias`, the
    /// `#heading`, an `![[` embed marker, and a Markdown link's text and title
    /// untouched. `relink` is what needs it; `link_graph` ignores it.
    pub span: Range<usize>,
}

/// The byte offset of `part` within `whole`, where `part` is a subslice of it.
///
/// Every narrowing this module does — `split`, `trim`, `strip_prefix` — yields a
/// subslice of its input, so the two pointers are into one allocation and the
/// difference between them is an offset. `str::substr_range` says the same thing
/// and is newer than this crate's MSRV.
fn offset_of(whole: &str, part: &str) -> usize {
    (part.as_ptr() as usize) - (whole.as_ptr() as usize)
}

/// Every link in one file, in the order they appear.
pub(crate) fn scan(text: &str) -> Vec<RawLink> {
    // Offsets are computed against the stripped text and shifted back at the
    // end, so a caller splicing by span never has to know a BOM was there.
    let bom = text.len() - text.trim_start_matches('\u{feff}').len();
    let text = &text[bom..];
    let mut links = Vec::new();

    let body_start = match frontmatter_end(text) {
        Some(end) => {
            // Wikilinks only inside frontmatter, and no code-fence suppression:
            // this is a YAML block, and its `links:` list is exactly the shape
            // the vault's own `types.json` registers as `multitext`.
            // Scanning the whole block rather than one known key catches
            // `related:`, `up:`, and any inline reference too.
            scan_wikilinks(&text[..end], 0, &mut links);
            end
        }
        None => 0,
    };

    let mut fence: Option<Fence> = None;
    let mut offset = body_start;
    for line in text[body_start..].split('\n') {
        // The separator `split` removed, so the next line starts one past this
        // one. A `\r` sits inside the line, so stripping it moves no offset.
        let stride = line.len() + 1;
        let line = line.strip_suffix('\r').unwrap_or(line);

        match &fence {
            Some(open) => {
                if closes(line, open) {
                    fence = None;
                }
                offset += stride;
                continue;
            }
            None => {
                if let Some(open) = opens(line) {
                    fence = Some(open);
                    offset += stride;
                    continue;
                }
            }
        }

        // Masking preserves byte length, so an offset into the masked line is
        // the same offset into the original one.
        scan_line(&mask_inline_code(line), offset, &mut links);
        offset += stride;
    }

    for link in &mut links {
        // Take the target from the original text rather than from the masked
        // line it was found in, so a span always slices back to exactly what
        // was written. The two differ only for a target straddling a code span
        // — ``[[a`b`c]]`` — which resolves to nothing either way, but the
        // invariant is what a caller splicing by span is entitled to rely on.
        let written = &text[link.span.clone()];
        link.target = match link.style {
            LinkStyle::Wiki => written.to_string(),
            LinkStyle::Markdown => percent_decode(written),
        };
        link.span.start += bom;
        link.span.end += bom;
    }

    links
}

/// The byte offset just past a leading `---` block, if there is one.
fn frontmatter_end(text: &str) -> Option<usize> {
    let mut offset = 0usize;
    let mut lines = text.split('\n');

    let first = lines.next()?;
    offset += first.len() + 1;
    if first.strip_suffix('\r').unwrap_or(first) != "---" {
        return None;
    }

    for line in lines {
        let end = offset + line.len() + 1;
        if line.strip_suffix('\r').unwrap_or(line) == "---" {
            return Some(end.min(text.len()));
        }
        offset = end;
    }
    // An unterminated block is not a block: a note whose body opens with a
    // horizontal rule must not have the rest of itself treated as YAML.
    None
}

/// An open fenced code block.
struct Fence {
    character: u8,
    length: usize,
}

fn opens(line: &str) -> Option<Fence> {
    let trimmed = line.trim_start_matches(' ');
    if line.len() - trimmed.len() > 3 {
        return None;
    }
    let character = match trimmed.as_bytes().first() {
        Some(&b'`') => b'`',
        Some(&b'~') => b'~',
        _ => return None,
    };
    let length = trimmed.bytes().take_while(|byte| *byte == character).count();
    if length < 3 {
        return None;
    }
    // An info string may not contain a backtick, or ``` in a sentence would
    // open a block.
    if character == b'`' && trimmed[length..].contains('`') {
        return None;
    }
    Some(Fence { character, length })
}

fn closes(line: &str, open: &Fence) -> bool {
    let trimmed = line.trim_start_matches(' ');
    if line.len() - trimmed.len() > 3 {
        return false;
    }
    let run = trimmed
        .bytes()
        .take_while(|byte| *byte == open.character)
        .count();
    // At least as long as the opener, and nothing but whitespace after it.
    run >= open.length && trimmed[run..].trim().is_empty()
}

/// Blank out inline code spans, keeping everything else verbatim.
///
/// A run of N backticks opens a span that closes at the next run of exactly N.
/// A run with no partner later on the line is ordinary text, so ``a ` b [[x]]``
/// still yields its link.
fn mask_inline_code(line: &str) -> String {
    let bytes = line.as_bytes();
    let mut masked = String::with_capacity(line.len());
    let mut index = 0usize;

    while index < bytes.len() {
        if bytes[index] != b'`' {
            let character = line[index..].chars().next().expect("a char boundary");
            masked.push(character);
            index += character.len_utf8();
            continue;
        }

        let run = bytes[index..].iter().take_while(|byte| **byte == b'`').count();
        match closing_run(bytes, index + run, run) {
            Some(close) => {
                // Blank the whole span, delimiters included, preserving length
                // so nothing downstream has to care that it was rewritten.
                masked.push_str(&" ".repeat(close + run - index));
                index = close + run;
            }
            None => {
                masked.push_str(&"`".repeat(run));
                index += run;
            }
        }
    }

    masked
}

/// Where a run of exactly `length` backticks starts, at or after `from`.
fn closing_run(bytes: &[u8], from: usize, length: usize) -> Option<usize> {
    let mut index = from;
    while index < bytes.len() {
        if bytes[index] != b'`' {
            index += 1;
            continue;
        }
        let run = bytes[index..].iter().take_while(|byte| **byte == b'`').count();
        if run == length {
            return Some(index);
        }
        index += run;
    }
    None
}

fn scan_wikilinks(text: &str, base: usize, links: &mut Vec<RawLink>) {
    let mut rest = text;
    while let Some(start) = rest.find("[[") {
        let after = &rest[start + 2..];
        let Some(end) = after.find("]]") else { break };
        push_wiki(&after[..end], base + offset_of(text, after), links);
        rest = &after[end + 2..];
    }
}

/// One line, already masked, scanned for both link styles.
fn scan_line(line: &str, base: usize, links: &mut Vec<RawLink>) {
    let bytes = line.as_bytes();
    let mut index = 0usize;

    while index < bytes.len() {
        if bytes[index] != b'[' {
            index += 1;
            continue;
        }

        // `[[target]]` — checked first, because a wikilink also starts with `[`.
        if bytes.get(index + 1) == Some(&b'[') {
            let after = &line[index + 2..];
            match after.find("]]") {
                Some(end) => {
                    push_wiki(&after[..end], base + index + 2, links);
                    index += 2 + end + 2;
                }
                None => index += 2,
            }
            continue;
        }

        // `[text](target)`
        let after = &line[index + 1..];
        let Some(close) = after.find(']') else {
            index += 1;
            continue;
        };
        if after.as_bytes().get(close + 1) != Some(&b'(') {
            index += 1;
            continue;
        }
        let destination = &after[close + 2..];
        match destination.find(')') {
            Some(end) => {
                push_markdown(&destination[..end], base + index + 1 + close + 2, links);
                index += 1 + close + 2 + end + 1;
            }
            None => index += 1,
        }
    }
}

fn push_wiki(inner: &str, base: usize, links: &mut Vec<RawLink>) {
    // `[[target|alias]]`, then `[[target#heading]]` and `[[target#^block]]`.
    let target = inner.split('|').next().unwrap_or_default();
    let target = target.split('#').next().unwrap_or_default().trim();
    // `[[#heading]]` points inside the same note and is not an edge.
    if target.is_empty() {
        return;
    }
    let start = base + offset_of(inner, target);
    links.push(RawLink {
        target: target.to_string(),
        style: LinkStyle::Wiki,
        span: start..start + target.len(),
    });
}

fn push_markdown(destination: &str, base: usize, links: &mut Vec<RawLink>) {
    let mut target = destination.trim();

    // An optional title: `[text](path "Title")`.
    if let Some(quote) = target.find(['"', '\'']) {
        target = target[..quote].trim();
    }
    // Angle-bracket form: `[text](<path with spaces>)`.
    target = target
        .strip_prefix('<')
        .map(|inner| inner.strip_suffix('>').unwrap_or(inner))
        .unwrap_or(target);
    target = target.split('#').next().unwrap_or_default().trim();

    if target.is_empty() || target.contains("://") || target.starts_with("mailto:") {
        return;
    }

    // The span covers the written form: `percent_decode` allocates, so the
    // decoded target has no offsets of its own to give.
    let start = base + offset_of(destination, target);
    links.push(RawLink {
        target: percent_decode(target),
        style: LinkStyle::Markdown,
        span: start..start + target.len(),
    });
}

/// Decode `%XX` escapes, which editors write for spaces in Markdown links.
///
/// Works on bytes throughout. Slicing the original `&str` by byte offset would
/// panic whenever a `%` sits close enough to a multi-byte character for the
/// three-byte escape window to land inside it — and a panic here takes the whole
/// vault index with it, over one link in one note.
///
/// Falls back to the raw text when the result is not valid UTF-8: a mangled
/// target is better reported as unresolved than as a failure.
fn percent_decode(raw: &str) -> String {
    if !raw.contains('%') {
        return raw.to_string();
    }

    let bytes = raw.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0usize;
    while index < bytes.len() {
        let escape = bytes
            .get(index + 1)
            .zip(bytes.get(index + 2))
            .filter(|_| bytes[index] == b'%')
            .and_then(|(high, low)| Some(hex(*high)? * 16 + hex(*low)?));

        match escape {
            Some(byte) => {
                decoded.push(byte);
                index += 3;
            }
            None => {
                decoded.push(bytes[index]);
                index += 1;
            }
        }
    }

    String::from_utf8(decoded).unwrap_or_else(|_| raw.to_string())
}

fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wiki(text: &str) -> Vec<String> {
        scan(text)
            .into_iter()
            .filter(|link| link.style == LinkStyle::Wiki)
            .map(|link| link.target)
            .collect()
    }

    fn markdown(text: &str) -> Vec<String> {
        scan(text)
            .into_iter()
            .filter(|link| link.style == LinkStyle::Markdown)
            .map(|link| link.target)
            .collect()
    }

    #[test]
    fn every_wikilink_form_reduces_to_the_target() {
        let text = "[[plain]] [[target|alias]] [[target#heading]] [[target#^block]] [[folder/target]] ![[embed]]";
        assert_eq!(
            wiki(text),
            ["plain", "target", "target", "target", "folder/target", "embed"]
        );
    }

    #[test]
    fn a_link_to_a_heading_in_the_same_note_is_not_an_edge() {
        assert!(wiki("See [[#Later section]].").is_empty());
    }

    #[test]
    fn fenced_code_suppresses_links() {
        let text = "before [[a]]\n```mermaid\ngraph TD\n  A[\"[[not_a_link]]\"]\n```\nafter [[b]]\n";
        assert_eq!(wiki(text), ["a", "b"]);
    }

    #[test]
    fn a_tilde_fence_and_an_info_string_both_suppress() {
        let text = "~~~python\n[[hidden]]\n~~~\n[[visible]]\n";
        assert_eq!(wiki(text), ["visible"]);
    }

    #[test]
    fn a_shorter_backtick_run_does_not_close_a_longer_fence() {
        let text = "````\n```\n[[still_hidden]]\n```\n````\n[[free]]\n";
        assert_eq!(wiki(text), ["free"]);
    }

    #[test]
    fn an_unclosed_fence_runs_to_the_end_of_the_file() {
        // A renderer does the same, so the graph should agree with what the
        // reader sees.
        assert!(wiki("```\n[[hidden]]\n").is_empty());
    }

    #[test]
    fn inline_code_suppresses_links() {
        assert_eq!(wiki("write `[[literal]]` or [[real]]"), ["real"]);
    }

    #[test]
    fn an_unmatched_backtick_does_not_swallow_the_rest_of_the_line() {
        assert_eq!(wiki("a ` b [[still_here]]"), ["still_here"]);
    }

    #[test]
    fn a_double_backtick_span_can_contain_a_single_one() {
        assert_eq!(wiki("``a ` [[hidden]]`` then [[shown]]"), ["shown"]);
    }

    #[test]
    fn frontmatter_links_are_edges() {
        // The exact shape a vault with `links` registered as multitext produces.
        let text = "---\nlinks:\n  - \"[[profile]]\"\n  - \"[[articles]]\"\n---\n\n# Body\n";
        assert_eq!(wiki(text), ["profile", "articles"]);
    }

    #[test]
    fn an_unterminated_leading_fence_is_body_not_frontmatter() {
        let text = "---\nnot really yaml\n\n[[still_a_link]]\n";
        assert_eq!(wiki(text), ["still_a_link"]);
    }

    #[test]
    fn markdown_links_are_edges_and_external_ones_are_not() {
        let text = "[a](notes/one.md) [b](https://example.com) [c](mailto:x@y.z) [d](#anchor) [e](<my note.md>)";
        assert_eq!(markdown(text), ["notes/one.md", "my note.md"]);
    }

    #[test]
    fn a_percent_encoded_markdown_target_is_decoded() {
        assert_eq!(markdown("[x](my%20note.md)"), ["my note.md"]);
    }

    #[test]
    fn a_markdown_title_is_not_part_of_the_target() {
        assert_eq!(markdown("[x](note.md \"A title\")"), ["note.md"]);
    }

    #[test]
    fn multibyte_text_around_a_code_span_survives_masking() {
        assert_eq!(wiki("café `code` [[naïve]] 日本語"), ["naïve"]);
    }

    #[test]
    fn a_percent_next_to_a_multibyte_character_does_not_panic() {
        // The escape window can land inside a multi-byte character. Slicing the
        // string by byte offset panics there, and a panic in the scanner takes
        // the whole vault index down over one link in one note.
        assert_eq!(markdown("[x](%aé.md)"), ["%aé.md"]);
        assert_eq!(markdown("[x](%é)"), ["%é"]);
        assert_eq!(markdown("[x](caf%C3%A9.md)"), ["café.md"]);
        assert_eq!(markdown("[x](trailing%)"), ["trailing%"]);
        assert_eq!(markdown("[x](%zz.md)"), ["%zz.md"]);
    }

    #[test]
    fn an_unterminated_wikilink_does_not_consume_the_line() {
        assert_eq!(wiki("[[broken and [[good]]"), ["broken and [[good"]);
    }

    /// Every span slices the text back to the target exactly as it was written.
    ///
    /// This is the whole basis of `relink`'s splice: it replaces a span and
    /// leaves the delimiters, the alias, the anchor and a Markdown link's text
    /// where they are. A span off by one byte corrupts a note.
    fn spans_slice_back(text: &str) {
        for link in scan(text) {
            let written = &text[link.span.clone()];
            let expected = match link.style {
                LinkStyle::Wiki => written.to_string(),
                LinkStyle::Markdown => percent_decode(written),
            };
            assert_eq!(expected, link.target, "span {:?} in {text:?}", link.span);
        }
    }

    #[test]
    fn every_span_slices_back_to_its_target() {
        for text in [
            "[[plain]] [[target|alias]] [[target#heading]] [[folder/target]] ![[embed]]",
            "---\nlinks:\n  - \"[[profile]]\"\n  - \"[[articles]]\"\n---\n\n# Body [[after]]\n",
            "before [[a]]\n```mermaid\n[[not_a_link]]\n```\nafter [[b]]\n",
            "write `[[literal]]` or [[real]]",
            "café `code` [[naïve]] 日本語 and [[another]]",
            "[a](notes/one.md) [e](<my note.md>) [f](caf%C3%A9.md) [g](note.md \"A title\")",
            "one\r\ntwo [[crlf]]\r\nthree\r\n",
            "\u{feff}[[after_a_bom]] and [[a_second]]",
            "  [[indented]]\n\n    [[four_spaces_is_not_code]]\n",
        ] {
            spans_slice_back(text);
        }
    }

    #[test]
    fn a_span_survives_a_masked_code_span_earlier_on_the_line() {
        // Masking is byte-length preserving, which is the property that lets a
        // masked-line offset be used against the original text.
        let links = scan("`x` and [[target]]");
        assert_eq!(links.len(), 1);
        assert_eq!(&"`x` and [[target]]"[links[0].span.clone()], "target");
    }

    #[test]
    fn a_span_is_measured_past_a_byte_order_mark() {
        let text = "\u{feff}# Title\n\n[[target]]\n";
        let links = scan(text);
        assert_eq!(links.len(), 1);
        assert_eq!(&text[links[0].span.clone()], "target");
    }

    #[test]
    fn a_markdown_span_covers_the_written_form_not_the_decoded_one() {
        let text = "[x](my%20note.md)";
        let links = scan(text);
        assert_eq!(links[0].target, "my note.md");
        assert_eq!(&text[links[0].span.clone()], "my%20note.md");
    }
}

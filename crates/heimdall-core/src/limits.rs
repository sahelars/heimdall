//! Every bound from SPEC §8 and §10, plus the validators that apply them.
//!
//! A request that exceeds a documented maximum is rejected with
//! `LIMIT_EXCEEDED` rather than silently clamped: a caller that asked for 5,000
//! lines and received 1,000 should be told, not left to infer it.

use crate::errors::{Error, Result};

// Listings (SPEC §8).
pub const LIST_LIMIT_DEFAULT: usize = 50;
pub const LIST_LIMIT_MAX: usize = 200;
pub const LIST_SCAN_GUARD: usize = 10_000;
pub const RECURSE_DEPTH_DEFAULT: usize = 4;
pub const RECURSE_DEPTH_MAX: usize = 16;

// Reads (SPEC §8, §10).
pub const READ_MAX_DOCUMENTS: usize = 10;
pub const READ_LINES_DEFAULT: usize = 200;
pub const READ_LINES_MAX: usize = 1_000;
pub const READ_TOTAL_BYTES_DEFAULT: usize = 64 * 1024;
pub const READ_TOTAL_BYTES_MAX: usize = 256 * 1024;

// Writes (SPEC §10).
/// The largest ordinary note the desktop client may save.
///
/// Matched to `STDIN_MAX_BYTES` below: content arrives on stdin, so a larger
/// cap here would only be rejected one layer down with a less useful message.
pub const DOCUMENT_MAX_BYTES: usize = 1024 * 1024;
pub const MAIN_MEMORY_MAX_BYTES: usize = 32 * 1024;
pub const MAIN_MEMORY_WARN_BYTES: usize = 24 * 1024;
pub const EXTENDED_MEMORY_MAX_BYTES: usize = 1024 * 1024;
pub const ENTRY_CONTENT_MAX_BYTES: usize = 1024 * 1024;

// Link graph (SPEC §8). The one operation that reads the whole vault, so its
// bounds are caps rather than pagination.
/// The most notes one graph carries. Past this, a force layout is unreadable
/// anyway, and the response reports what it left out.
pub const GRAPH_MAX_NODES: usize = 5_000;
/// The most of any single file that is scanned for links. A note larger than a
/// whole `read_documents` request is not a note.
pub const GRAPH_MAX_FILE_BYTES: u64 = READ_TOTAL_BYTES_MAX as u64;
/// The total scan budget: roughly `GRAPH_MAX_NODES` files of average size, and
/// the wall that stops a vault full of pasted logs.
pub const GRAPH_MAX_TOTAL_BYTES: u64 = 64 * 1024 * 1024;
/// The most links extracted from one file, so a generated index page cannot
/// dominate the response on its own.
pub const GRAPH_MAX_LINKS_PER_FILE: usize = 1_000;

/// The largest agent instructions file the client may save.
///
/// `AGENTS.md` is always-in-context material like the main memory — the shipped
/// template's own first line is "MAX 10K TOKENS" — so it gets a bound of the
/// same character, well above any reasonable instructions file and far below
/// anything that would bloat a client's context.
pub const AGENTS_MAX_BYTES: usize = 64 * 1024;

/// The largest body any single stdin read will accept, used by the CLI to bound
/// input before it reaches a domain limit check.
pub const STDIN_MAX_BYTES: usize = EXTENDED_MEMORY_MAX_BYTES;

fn resolve(
    requested: Option<u32>,
    default: usize,
    max: usize,
    name: &str,
) -> Result<usize> {
    match requested {
        None => Ok(default),
        Some(0) => Err(Error::invalid_input(format!("{name} must be at least 1"))
            .with_detail("parameter", name)),
        Some(value) => {
            let value = value as usize;
            if value > max {
                return Err(Error::limit_exceeded(format!(
                    "{name} is {value}, above the maximum of {max}"
                ))
                .with_detail("parameter", name)
                .with_detail("requested", value)
                .with_detail("maximum", max));
            }
            Ok(value)
        }
    }
}

/// Page size for any listing operation.
pub fn resolve_limit(requested: Option<u32>) -> Result<usize> {
    resolve(requested, LIST_LIMIT_DEFAULT, LIST_LIMIT_MAX, "limit")
}

/// Lines returned for one file in one read.
pub fn resolve_max_lines(requested: Option<u32>) -> Result<usize> {
    resolve(requested, READ_LINES_DEFAULT, READ_LINES_MAX, "max_lines")
}

/// Depth for a whole-vault index.
///
/// Unlike a listing, this defaults to the maximum. `link_graph` is the vault's
/// whole link structure, and a graph that quietly stopped four folders down
/// would report real links as unresolved rather than admitting it had not
/// looked — the one thing the truncation flags exist to prevent.
pub fn resolve_graph_depth(requested: Option<u32>) -> Result<usize> {
    resolve(
        requested,
        RECURSE_DEPTH_MAX,
        RECURSE_DEPTH_MAX,
        "max_depth",
    )
}

/// Recursion depth for `list_documents`.
pub fn resolve_max_depth(requested: Option<u32>) -> Result<usize> {
    resolve(
        requested,
        RECURSE_DEPTH_DEFAULT,
        RECURSE_DEPTH_MAX,
        "max_depth",
    )
}

/// Total content budget across every file in one read request.
pub fn resolve_max_total_bytes(requested: Option<u32>) -> Result<usize> {
    resolve(
        requested,
        READ_TOTAL_BYTES_DEFAULT,
        READ_TOTAL_BYTES_MAX,
        "max_total_bytes",
    )
}

/// The 1-indexed first line of a read.
pub fn resolve_start_line(requested: Option<u32>) -> Result<usize> {
    match requested {
        None => Ok(1),
        Some(0) => Err(Error::invalid_input("start_line is 1-indexed and must be at least 1")
            .with_detail("parameter", "start_line")),
        Some(value) => Ok(value as usize),
    }
}

/// Reject a write whose body is larger than its destination allows.
pub fn check_write_size(actual: usize, max: usize, target: &str) -> Result<()> {
    if actual > max {
        return Err(Error::limit_exceeded(format!(
            "{target} content is {actual} bytes, above the limit of {max} bytes"
        ))
        .with_detail("target", target)
        .with_detail("size_bytes", actual)
        .with_detail("maximum", max));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::ErrorCode;

    #[test]
    fn absent_values_take_the_documented_default() {
        assert_eq!(resolve_limit(None).unwrap(), 50);
        assert_eq!(resolve_max_lines(None).unwrap(), 200);
        assert_eq!(resolve_max_depth(None).unwrap(), 4);
        assert_eq!(resolve_max_total_bytes(None).unwrap(), 65_536);
        assert_eq!(resolve_start_line(None).unwrap(), 1);
    }

    #[test]
    fn a_whole_vault_index_defaults_to_full_depth_rather_than_a_listing_page() {
        assert_eq!(resolve_graph_depth(None).unwrap(), RECURSE_DEPTH_MAX);
        assert_eq!(resolve_graph_depth(Some(2)).unwrap(), 2);
        assert_eq!(
            resolve_graph_depth(Some(RECURSE_DEPTH_MAX as u32 + 1))
                .unwrap_err()
                .code,
            ErrorCode::LimitExceeded
        );
    }

    #[test]
    fn maximums_are_accepted_and_anything_above_is_rejected() {
        assert_eq!(resolve_limit(Some(200)).unwrap(), 200);
        assert_eq!(
            resolve_limit(Some(201)).unwrap_err().code,
            ErrorCode::LimitExceeded
        );
        assert_eq!(resolve_max_lines(Some(1_000)).unwrap(), 1_000);
        assert_eq!(
            resolve_max_lines(Some(1_001)).unwrap_err().code,
            ErrorCode::LimitExceeded
        );
        assert_eq!(resolve_max_depth(Some(16)).unwrap(), 16);
        assert_eq!(
            resolve_max_depth(Some(17)).unwrap_err().code,
            ErrorCode::LimitExceeded
        );
        assert_eq!(resolve_max_total_bytes(Some(262_144)).unwrap(), 262_144);
        assert_eq!(
            resolve_max_total_bytes(Some(262_145)).unwrap_err().code,
            ErrorCode::LimitExceeded
        );
    }

    #[test]
    fn zero_is_a_caller_mistake_not_a_limit_breach() {
        assert_eq!(
            resolve_limit(Some(0)).unwrap_err().code,
            ErrorCode::InvalidInput
        );
        assert_eq!(
            resolve_start_line(Some(0)).unwrap_err().code,
            ErrorCode::InvalidInput
        );
    }

    #[test]
    fn write_size_check_reports_the_breached_limit() {
        assert!(check_write_size(MAIN_MEMORY_MAX_BYTES, MAIN_MEMORY_MAX_BYTES, "main memory").is_ok());
        let err = check_write_size(
            MAIN_MEMORY_MAX_BYTES + 1,
            MAIN_MEMORY_MAX_BYTES,
            "main memory",
        )
        .unwrap_err();
        assert_eq!(err.code, ErrorCode::LimitExceeded);
        assert_eq!(err.details["maximum"], 32_768);
    }
}

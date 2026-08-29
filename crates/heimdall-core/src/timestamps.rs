//! UTC time formatting.
//!
//! Entry filenames and `created_at` frontmatter derive from the same UTC
//! instant (SPEC §6), so they cannot disagree or misorder across DST changes.

use cap_std::time::{SystemClock, SystemTime as CapSystemTime};
use time::format_description::FormatItem;
use time::macros::format_description;
use time::OffsetDateTime;

use crate::errors::{Error, Result};

/// RFC 3339 in UTC at second precision, e.g. `2026-08-16T14:30:00Z`.
const RFC3339_UTC: &[FormatItem<'_>] =
    format_description!("[year]-[month]-[day]T[hour]:[minute]:[second]Z");

/// Entry filename stem, e.g. `2026-08-16_14-30-00`.
const ENTRY_STEM: &[FormatItem<'_>] =
    format_description!("[year]-[month]-[day]_[hour]-[minute]-[second]");

pub fn now_utc() -> OffsetDateTime {
    OffsetDateTime::now_utc()
}

pub fn to_rfc3339(at: OffsetDateTime) -> String {
    at.format(RFC3339_UTC)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

/// The `YYYY-MM-DD_HH-mm-ss` filename stem for an entry created at `at`.
pub fn entry_stem(at: OffsetDateTime) -> String {
    at.format(ENTRY_STEM)
        .unwrap_or_else(|_| "1970-01-01_00-00-00".to_string())
}

/// Recover the creation instant from an entry filename.
///
/// Accepts `YYYY-MM-DD_HH-mm-ss.md` and its collision variants
/// (`..._01.md`). Returns `None` for any other name — entry folders may hold
/// files a user created by hand in any editor, and those still have to list.
pub fn parse_entry_stem(file_name: &str) -> Option<OffsetDateTime> {
    let stem = file_name.strip_suffix(".md").unwrap_or(file_name);
    // A collision suffix is `_NN` after the fixed-width timestamp.
    let stem = if stem.len() > 19 { &stem[..19] } else { stem };
    let parsed = time::PrimitiveDateTime::parse(stem, ENTRY_STEM).ok()?;
    Some(parsed.assume_utc())
}

/// Convert a capability-scoped filesystem timestamp into UTC.
pub fn from_cap(at: CapSystemTime) -> Result<OffsetDateTime> {
    let since_epoch = at
        .duration_since(SystemClock::UNIX_EPOCH)
        .map_err(|_| Error::io_error("file timestamp precedes the Unix epoch"))?;
    Ok(OffsetDateTime::UNIX_EPOCH + since_epoch)
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    #[test]
    fn timestamps_render_in_utc_at_second_precision() {
        let at = datetime!(2026-08-16 14:30:00 UTC);
        assert_eq!(to_rfc3339(at), "2026-08-16T14:30:00Z");
        assert_eq!(entry_stem(at), "2026-08-16_14-30-00");
    }

    #[test]
    fn filename_and_frontmatter_derive_from_one_instant() {
        let at = datetime!(2026-01-02 03:04:05.999 UTC);
        // Sub-second detail is dropped identically on both sides.
        assert_eq!(entry_stem(at), "2026-01-02_03-04-05");
        assert_eq!(to_rfc3339(at), "2026-01-02T03:04:05Z");
    }

    #[test]
    fn entry_filenames_round_trip_back_to_their_instant() {
        let at = datetime!(2026-08-16 10:30:00 UTC);
        let name = format!("{}.md", entry_stem(at));
        assert_eq!(parse_entry_stem(&name), Some(at));
        // Collision variants carry the same instant.
        assert_eq!(parse_entry_stem("2026-08-16_10-30-00_01.md"), Some(at));
    }

    #[test]
    fn hand_made_filenames_simply_have_no_recoverable_instant() {
        for name in ["2026_8_13.md", "notes.md", "", "2026-13-45_99-99-99.md"] {
            assert_eq!(parse_entry_stem(name), None, "{name}");
        }
    }

    #[test]
    fn entry_stems_sort_chronologically_as_plain_strings() {
        let earlier = entry_stem(datetime!(2026-08-16 09:59:59 UTC));
        let later = entry_stem(datetime!(2026-08-16 10:00:00 UTC));
        assert!(earlier < later, "{earlier} !< {later}");
    }
}

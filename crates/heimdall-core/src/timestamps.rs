//! UTC time formatting. Every timestamp Heimdall reports is UTC, so two
//! machines in different zones report one modification time the same way.

use cap_std::time::{SystemClock, SystemTime as CapSystemTime};
use time::format_description::FormatItem;
use time::macros::format_description;
use time::OffsetDateTime;

use crate::errors::{Error, Result};

/// RFC 3339 in UTC at second precision, e.g. `2026-08-16T14:30:00Z`.
const RFC3339_UTC: &[FormatItem<'_>] =
    format_description!("[year]-[month]-[day]T[hour]:[minute]:[second]Z");

pub fn to_rfc3339(at: OffsetDateTime) -> String {
    at.format(RFC3339_UTC)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
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
        // Sub-second detail is dropped rather than rounded.
        assert_eq!(to_rfc3339(datetime!(2026-01-02 03:04:05.999 UTC)), "2026-01-02T03:04:05Z");
    }
}

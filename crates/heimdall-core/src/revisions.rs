//! Content revisions: `blake3:<lowercase-hex>` over the exact stored bytes.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::errors::{Error, Result};

const PREFIX: &str = "blake3:";
const HEX_LEN: usize = 64;

/// An opaque content revision. Compared for equality, never ordered.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Revision(String);

impl Revision {
    /// Hash the exact bytes as stored on disk.
    pub fn of_bytes(bytes: &[u8]) -> Self {
        Self(format!("{PREFIX}{}", blake3::hash(bytes).to_hex()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for Revision {
    type Err = Error;

    fn from_str(raw: &str) -> Result<Self> {
        let Some(hex) = raw.strip_prefix(PREFIX) else {
            return Err(Error::invalid_input(
                "expected_revision must start with \"blake3:\"",
            )
            .with_detail("parameter", "expected_revision"));
        };
        if hex.len() != HEX_LEN || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(Error::invalid_input(format!(
                "expected_revision must be \"blake3:\" followed by {HEX_LEN} hex characters"
            ))
            .with_detail("parameter", "expected_revision"));
        }
        if hex.bytes().any(|b| b.is_ascii_uppercase()) {
            return Err(Error::invalid_input(
                "expected_revision hex must be lowercase",
            )
            .with_detail("parameter", "expected_revision"));
        }
        Ok(Self(raw.to_string()))
    }
}

impl fmt::Display for Revision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Serialize for Revision {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for Revision {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Self::from_str(&raw).map_err(serde::de::Error::custom)
    }
}

/// A revision is an opaque string on the wire, so tool schemas describe it as
/// one rather than exposing the newtype as an object.
impl schemars::JsonSchema for Revision {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "Revision".into()
    }

    fn json_schema(_generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "string",
            "pattern": format!("^{PREFIX}[0-9a-f]{{{HEX_LEN}}}$"),
            "description": "Content revision of the exact stored bytes, as \
                            \"blake3:\" followed by 64 lowercase hex characters.",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn revisions_are_prefixed_lowercase_hex_of_exact_bytes() {
        let revision = Revision::of_bytes(b"# Memory\n");
        let hex = revision.as_str().strip_prefix(PREFIX).unwrap();
        assert_eq!(hex.len(), HEX_LEN);
        assert!(hex.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    }

    #[test]
    fn one_changed_byte_changes_the_revision() {
        assert_ne!(Revision::of_bytes(b"a"), Revision::of_bytes(b"b"));
        assert_eq!(Revision::of_bytes(b"a"), Revision::of_bytes(b"a"));
        // Trailing newlines are content, not formatting.
        assert_ne!(Revision::of_bytes(b"a"), Revision::of_bytes(b"a\n"));
    }

    #[test]
    fn round_trips_through_parsing() {
        let revision = Revision::of_bytes(b"content");
        assert_eq!(Revision::from_str(revision.as_str()).unwrap(), revision);
    }

    #[test]
    fn malformed_revisions_are_invalid_input() {
        for raw in [
            "",
            "deadbeef",
            "sha256:0000000000000000000000000000000000000000000000000000000000000000",
            "blake3:short",
            "blake3:zz00000000000000000000000000000000000000000000000000000000000000",
        ] {
            assert!(Revision::from_str(raw).is_err(), "accepted {raw:?}");
        }
    }

    #[test]
    fn uppercase_hex_is_rejected_so_comparison_stays_exact() {
        let upper = format!("{PREFIX}{}", "A".repeat(HEX_LEN));
        assert!(Revision::from_str(&upper).is_err());
    }
}

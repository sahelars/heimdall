//! The fixed domain error set (SPEC §11).
//!
//! Errors carry a stable machine code, a human message, and optional structured
//! details. Messages never contain absolute paths or stack traces (SPEC §14);
//! callers pass vault-relative context only.

use std::io;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// The complete set of domain error codes. Adding a code is a spec change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    InvalidInput,
    LimitExceeded,
    PathOutsideVault,
    NotFound,
    AlreadyExists,
    NotInitialized,
    RevisionConflict,
    Locked,
    IoError,
    InternalError,
}

impl ErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InvalidInput => "INVALID_INPUT",
            Self::LimitExceeded => "LIMIT_EXCEEDED",
            Self::PathOutsideVault => "PATH_OUTSIDE_VAULT",
            Self::NotFound => "NOT_FOUND",
            Self::AlreadyExists => "ALREADY_EXISTS",
            Self::NotInitialized => "NOT_INITIALIZED",
            Self::RevisionConflict => "REVISION_CONFLICT",
            Self::Locked => "LOCKED",
            Self::IoError => "IO_ERROR",
            Self::InternalError => "INTERNAL_ERROR",
        }
    }
}

impl std::fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A domain failure. Expected at process boundaries; never a panic.
#[derive(Debug, Clone, Serialize, thiserror::Error)]
#[error("{code}: {message}")]
pub struct Error {
    pub code: ErrorCode,
    pub message: String,
    #[serde(skip_serializing_if = "Map::is_empty")]
    pub details: Map<String, Value>,
}

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            details: Map::new(),
        }
    }

    /// Attach a structured detail. Details must stay safe to show a client:
    /// relative paths, limits, and codes — never absolute paths or content.
    #[must_use]
    pub fn with_detail(mut self, key: &str, value: impl Into<Value>) -> Self {
        self.details.insert(key.to_string(), value.into());
        self
    }

    pub fn invalid_input(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidInput, message)
    }

    pub fn limit_exceeded(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::LimitExceeded, message)
    }

    pub fn path_outside_vault(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::PathOutsideVault, message)
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::NotFound, message)
    }

    pub fn already_exists(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::AlreadyExists, message)
    }

    pub fn not_initialized(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::NotInitialized, message)
    }

    pub fn revision_conflict(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::RevisionConflict, message)
    }

    pub fn locked(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Locked, message)
    }

    pub fn io_error(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::IoError, message)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::InternalError, message)
    }

    /// Map an I/O failure from a data operation (read, write, rename).
    pub fn from_io(context: &str, err: &io::Error) -> Self {
        let code = match err.kind() {
            io::ErrorKind::NotFound => ErrorCode::NotFound,
            io::ErrorKind::AlreadyExists => ErrorCode::AlreadyExists,
            _ => ErrorCode::IoError,
        };
        Self::new(code, format!("{context}: {}", sanitize(err)))
    }

    /// Map an I/O failure from resolving a path through a directory capability.
    ///
    /// Inside a `cap-std` `Dir`, `PermissionDenied` is how an escape attempt
    /// surfaces — a symlink pointing out of the vault, or a component the
    /// capability refuses to traverse — so it becomes `PATH_OUTSIDE_VAULT`.
    pub fn from_io_path(context: &str, err: &io::Error) -> Self {
        if err.kind() == io::ErrorKind::PermissionDenied {
            return Self::path_outside_vault(format!(
                "{context}: path escapes the vault or cannot be traversed"
            ));
        }
        Self::from_io(context, err)
    }
}

/// Strip anything that could leak host layout out of an I/O message.
///
/// `io::Error` Display for OS errors is a bare description ("No such file or
/// directory"), but custom errors can carry payloads, so absolute paths are
/// replaced rather than trusted.
fn sanitize(err: &io::Error) -> String {
    let raw = match err.raw_os_error() {
        Some(_) => err.to_string(),
        None => err.kind().to_string(),
    };
    raw.split_whitespace()
        .map(|token| {
            if token.starts_with('/') || token.contains(":\\") {
                "<path>"
            } else {
                token
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_serialize_as_screaming_snake_case() {
        let json = serde_json::to_string(&ErrorCode::PathOutsideVault).unwrap();
        assert_eq!(json, "\"PATH_OUTSIDE_VAULT\"");
        assert_eq!(ErrorCode::RevisionConflict.as_str(), "REVISION_CONFLICT");
    }

    #[test]
    fn empty_details_are_omitted_from_json() {
        let err = Error::not_found("Document does not exist");
        let json = serde_json::to_value(&err).unwrap();
        assert!(json.get("details").is_none());

        let err = err.with_detail("path", "projects/a.md");
        let json = serde_json::to_value(&err).unwrap();
        assert_eq!(json["details"]["path"], "projects/a.md");
    }

    #[test]
    fn io_not_found_maps_to_domain_not_found() {
        let err = io::Error::new(io::ErrorKind::NotFound, "boom");
        assert_eq!(Error::from_io("read", &err).code, ErrorCode::NotFound);
    }

    #[test]
    fn capability_permission_denied_maps_to_path_outside_vault() {
        let err = io::Error::new(io::ErrorKind::PermissionDenied, "escape");
        assert_eq!(
            Error::from_io_path("open", &err).code,
            ErrorCode::PathOutsideVault
        );
        // The same error from a data operation stays an I/O error.
        assert_eq!(Error::from_io("write", &err).code, ErrorCode::IoError);
    }

    #[test]
    fn absolute_paths_are_redacted_from_messages() {
        let err = io::Error::other("failed at /Users/someone/Secret Vault/notes");
        let message = Error::from_io("read", &err).message;
        assert!(!message.contains("/Users/someone"), "{message}");
    }
}

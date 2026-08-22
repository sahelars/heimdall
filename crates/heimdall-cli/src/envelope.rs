//! The versioned shell output envelope (SPEC §11).
//!
//! Every shell command prints exactly one JSON object on stdout. Diagnostics go
//! to stderr so stdout stays parseable — the same discipline the MCP adapter
//! will need for protocol traffic in Phase 2.

use std::io::Write;
use std::process::ExitCode;

use serde::Serialize;
use serde_json::json;
use heimdall_core::Error;

/// Bumped only when the envelope shape itself changes.
pub const SCHEMA_VERSION: u32 = 1;

/// A domain failure. The envelope is still printed; only the status differs.
pub const EXIT_DOMAIN_ERROR: u8 = 1;

pub fn ok<T: Serialize>(data: &T) -> ExitCode {
    let body = json!({
        "ok": true,
        "data": data,
        "meta": { "schema_version": SCHEMA_VERSION },
    });
    print(&body);
    ExitCode::SUCCESS
}

pub fn failure(error: &Error) -> ExitCode {
    let body = json!({
        "ok": false,
        "error": {
            "code": error.code,
            "message": error.message,
            "details": error.details,
        },
        "meta": { "schema_version": SCHEMA_VERSION },
    });
    print(&body);
    ExitCode::from(EXIT_DOMAIN_ERROR)
}

fn print(body: &serde_json::Value) {
    let rendered = serde_json::to_string_pretty(body)
        .unwrap_or_else(|_| r#"{"ok":false,"error":{"code":"INTERNAL_ERROR","message":"failed to serialize response","details":{}},"meta":{"schema_version":1}}"#.to_string());

    let stdout = std::io::stdout();
    let mut handle = stdout.lock();
    if writeln!(handle, "{rendered}").is_err() {
        // A closed stdout is not something a second write can report.
        return;
    }
    let _ = handle.flush();
}

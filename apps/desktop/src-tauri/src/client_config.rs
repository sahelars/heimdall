//! Writing the MCP server entry into known client configuration files.
//!
//! The Server screen offers this, and only ever with explicit consent
//! (SPEC §15). React names a client from the detected list — never a path — so
//! this can only touch files this module already knows about, and an existing
//! configuration is merged rather than replaced.

use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::{json, Map, Value};

use crate::cli_bridge::{DomainError, SidecarOrigin};

/// The configuration files this application knows how to edit.
///
/// Each entry is a fixed, per-user location. Nothing here is derived from
/// anything React sends.
const KNOWN_CLIENTS: &[(&str, &str, &[&str])] = &[
    (
        "claude-desktop",
        "Claude Desktop",
        &["Library", "Application Support", "Claude", "claude_desktop_config.json"],
    ),
    ("cursor", "Cursor", &[".cursor", "mcp.json"]),
    (
        "vscode",
        "Visual Studio Code",
        &["Library", "Application Support", "Code", "User", "mcp.json"],
    ),
];

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KnownClient {
    pub id: String,
    pub name: String,
    pub path: String,
    /// Whether the configuration file exists today.
    pub present: bool,
    /// Whether it already has a Heimdall server entry for this vault.
    pub installed: bool,
    /// Whether that entry names a command that is no longer there.
    ///
    /// A client whose configured command cannot be spawned reports a timeout,
    /// never a missing file, so this is the only place the user can find out.
    pub stale: bool,
    /// The entry name that would be written.
    pub server_key: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallOutcome {
    pub path: String,
    pub server_key: String,
    /// Whether an existing entry of the same name was replaced.
    pub replaced: bool,
    /// Where the previous file was copied before writing, if there was one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backup_path: Option<String>,
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

fn config_path(segments: &[&str]) -> Option<PathBuf> {
    let mut path = home()?;
    for segment in segments {
        path.push(segment);
    }
    Some(path)
}

/// The `mcpServers` entry name to use for this vault.
///
/// `heimdall` is the plain name from the spec's example. When it is already
/// taken by a different vault the name is qualified with the vault's folder,
/// which is exactly the one-process-per-vault arrangement §7 describes — and it
/// means installing a second vault never quietly steals the first one's entry.
pub fn server_key(existing: &Value, vault: &str) -> String {
    let folder = Path::new(vault)
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "vault".to_string());

    let taken_by_other_vault = existing
        .get("mcpServers")
        .and_then(Value::as_object)
        .and_then(|servers| servers.get("heimdall"))
        .is_some_and(|entry| !entry_points_at(entry, vault));

    if taken_by_other_vault {
        format!("heimdall-{}", sanitize(&folder))
    } else {
        "heimdall".to_string()
    }
}

fn entry_points_at(entry: &Value, vault: &str) -> bool {
    entry
        .get("args")
        .and_then(Value::as_array)
        .is_some_and(|args| args.iter().any(|arg| arg.as_str() == Some(vault)))
}

/// Whether the command an existing entry names is still on disk.
///
/// Only an absolute path can be checked. A bare name is left alone: it is
/// resolved through the client's own `PATH`, which is not this process's.
fn entry_command_exists(entry: &Value) -> bool {
    let Some(command) = entry.get("command").and_then(Value::as_str) else {
        return true;
    };
    let path = Path::new(command);
    !path.is_absolute() || path.is_file()
}

/// Reduce a folder name to something safe to use as a JSON key.
fn sanitize(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect();
    let trimmed = cleaned.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "vault".to_string()
    } else {
        trimmed
    }
}

/// The server entry a client should run.
pub fn server_entry(command: &Path, vault: &str) -> Value {
    json!({
        "command": command.to_string_lossy(),
        "args": ["mcp", "--vault", vault],
    })
}

/// Which known clients are present, and whether each already points at this vault.
pub fn known_clients(vault: &str) -> Vec<KnownClient> {
    KNOWN_CLIENTS
        .iter()
        .filter_map(|(id, name, segments)| {
            let path = config_path(segments)?;
            let existing = read_json(&path).unwrap_or_else(|| json!({}));
            let key = server_key(&existing, vault);
            let entry = existing
                .get("mcpServers")
                .and_then(Value::as_object)
                .and_then(|servers| servers.get(&key))
                .filter(|entry| entry_points_at(entry, vault));
            let installed = entry.is_some();
            let stale = entry.is_some_and(|entry| !entry_command_exists(entry));

            Some(KnownClient {
                id: id.to_string(),
                name: name.to_string(),
                path: path.to_string_lossy().to_string(),
                present: path.is_file(),
                installed,
                stale,
                server_key: key,
            })
        })
        .collect()
}

fn read_json(path: &Path) -> Option<Value> {
    let text = std::fs::read_to_string(path).ok()?;
    if text.trim().is_empty() {
        return Some(json!({}));
    }
    serde_json::from_str(&text).ok()
}

/// Merge the server entry into one known client's configuration.
pub fn install(
    client_id: &str,
    vault: &str,
    command: &Path,
    origin: SidecarOrigin,
) -> Result<InstallOutcome, DomainError> {
    // Before anything is read, copied, or written. A client configuration is a
    // file the user keeps; a development build's CLI lives in a build directory
    // that a rebuild or `cargo clean` removes, and the failure that leaves
    // behind is invisible — a client whose command has gone reports a timeout,
    // never a missing file (SPEC §15).
    if origin.is_development() {
        return Err(DomainError::new(
            "INVALID_INPUT",
            "this is a development build, and its command line tool lives in a build directory \
             that a rebuild or \"cargo clean\" removes; a client whose configured command has \
             gone reports only a timeout, never a missing file. Install the built application \
             and register from there, or paste the snippet above into the client yourself",
        ));
    }

    let Some((_, _, segments)) = KNOWN_CLIENTS.iter().find(|(id, _, _)| *id == client_id) else {
        return Err(DomainError::new(
            "INVALID_INPUT",
            format!("\"{client_id}\" is not a client this application can configure"),
        ));
    };
    let path = config_path(segments).ok_or_else(|| {
        DomainError::new("IO_ERROR", "could not locate this user's home directory")
    })?;

    // A file that exists but is not JSON is someone's working configuration.
    // Overwriting it would destroy settings this application knows nothing
    // about, so it stops instead.
    let existing = if path.is_file() {
        read_json(&path).ok_or_else(|| {
            DomainError::new(
                "INVALID_INPUT",
                "that configuration file is not valid JSON; open it and fix it first",
            )
        })?
    } else {
        json!({})
    };

    let key = server_key(&existing, vault);
    let mut document = match existing {
        Value::Object(map) => map,
        _ => Map::new(),
    };

    let servers = document
        .entry("mcpServers".to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    let Some(servers) = servers.as_object_mut() else {
        return Err(DomainError::new(
            "INVALID_INPUT",
            "that configuration file has an \"mcpServers\" value that is not an object",
        ));
    };

    let replaced = servers.contains_key(&key);
    servers.insert(key.clone(), server_entry(command, vault));

    let backup_path = back_up(&path)?;
    write_atomically(&path, &Value::Object(document))?;

    Ok(InstallOutcome {
        path: path.to_string_lossy().to_string(),
        server_key: key,
        replaced,
        backup_path,
    })
}

/// Keep a copy of what was there before touching a file the user owns.
fn back_up(path: &Path) -> Result<Option<String>, DomainError> {
    if !path.is_file() {
        return Ok(None);
    }
    let backup = path.with_extension("heimdall-backup.json");
    std::fs::copy(path, &backup).map_err(|err| {
        DomainError::new(
            "IO_ERROR",
            format!("could not back up the existing configuration: {}", err.kind()),
        )
    })?;
    Ok(Some(backup.to_string_lossy().to_string()))
}

/// Write through a temporary sibling so an interrupted save cannot leave a
/// client with a half-written configuration.
fn write_atomically(path: &Path, document: &Value) -> Result<(), DomainError> {
    let parent = path.parent().ok_or_else(|| {
        DomainError::new("IO_ERROR", "that configuration path has no parent directory")
    })?;
    std::fs::create_dir_all(parent).map_err(|err| {
        DomainError::new(
            "IO_ERROR",
            format!("could not create the configuration directory: {}", err.kind()),
        )
    })?;

    let mut text = serde_json::to_string_pretty(document)
        .map_err(|_| DomainError::new("INTERNAL_ERROR", "could not render the configuration"))?;
    text.push('\n');

    let temp = path.with_extension("heimdall-tmp");
    std::fs::write(&temp, text).map_err(|err| {
        DomainError::new(
            "IO_ERROR",
            format!("could not write the configuration: {}", err.kind()),
        )
    })?;
    std::fs::rename(&temp, path).map_err(|err| {
        let _ = std::fs::remove_file(&temp);
        DomainError::new(
            "IO_ERROR",
            format!("could not save the configuration: {}", err.kind()),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_plain_name_is_used_when_nothing_claims_it() {
        assert_eq!(server_key(&json!({}), "/Users/n/Vault"), "heimdall");
        assert_eq!(
            server_key(&json!({ "mcpServers": { "other": {} } }), "/Users/n/Vault"),
            "heimdall"
        );
    }

    #[test]
    fn reinstalling_the_same_vault_reuses_its_entry() {
        let existing = json!({
            "mcpServers": {
                "heimdall": { "command": "/x/heimdall", "args": ["mcp", "--vault", "/Users/n/Vault"] }
            }
        });
        assert_eq!(server_key(&existing, "/Users/n/Vault"), "heimdall");
    }

    #[test]
    fn a_second_vault_gets_its_own_entry_instead_of_stealing_the_first() {
        let existing = json!({
            "mcpServers": {
                "heimdall": { "command": "/x/heimdall", "args": ["mcp", "--vault", "/Users/n/Personal"] }
            }
        });
        assert_eq!(server_key(&existing, "/Users/n/Work Notes"), "heimdall-work-notes");
    }

    #[test]
    fn folder_names_are_reduced_to_safe_keys() {
        assert_eq!(sanitize("My Vault"), "my-vault");
        assert_eq!(sanitize("notes.v2"), "notes-v2");
        assert_eq!(sanitize("---"), "vault");
        assert_eq!(sanitize(""), "vault");
    }

    #[test]
    fn the_entry_runs_the_bundled_command_with_the_vault_fixed() {
        let entry = server_entry(Path::new("/Applications/Heimdall.app/Contents/MacOS/heimdall"), "/v");
        assert_eq!(entry["command"], "/Applications/Heimdall.app/Contents/MacOS/heimdall");
        assert_eq!(entry["args"], json!(["mcp", "--vault", "/v"]));
    }

    #[test]
    fn only_known_clients_can_be_written_to() {
        let shipped = SidecarOrigin::Shipped;
        let error = install("/etc/passwd", "/v", Path::new("/x/heimdall"), shipped).unwrap_err();
        assert_eq!(error.code, "INVALID_INPUT");

        let error = install("../../evil", "/v", Path::new("/x/heimdall"), shipped).unwrap_err();
        assert_eq!(error.code, "INVALID_INPUT");
    }

    #[test]
    fn a_development_build_is_refused_before_anything_is_touched() {
        // The path a development build would write disappears on a rebuild, and
        // the client it was written for then reports a timeout rather than a
        // missing file — so the refusal has to explain itself (SPEC §15).
        let backup = config_path(KNOWN_CLIENTS[0].2)
            .expect("a home directory")
            .with_extension("heimdall-backup.json");
        let before = backup.is_file();

        let error = install(
            KNOWN_CLIENTS[0].0,
            "/v",
            Path::new("/x/target/debug/heimdall"),
            SidecarOrigin::Development,
        )
        .unwrap_err();

        assert_eq!(error.code, "INVALID_INPUT");
        assert!(error.message.contains("development build"), "{}", error.message);
        assert!(error.message.contains("build directory"), "{}", error.message);
        // The guard runs before the back-up, which is `install`'s first write.
        assert_eq!(backup.is_file(), before, "a refused install still wrote something");
    }

    #[test]
    fn a_registration_is_stale_only_when_its_absolute_command_is_gone() {
        let dir = tempfile::tempdir().unwrap();
        let present = dir.path().join("heimdall");
        std::fs::write(&present, b"#!/bin/sh\n").unwrap();

        let entry = |command: &str| json!({ "command": command, "args": ["mcp", "--vault", "/v"] });

        assert!(entry_command_exists(&entry(&present.to_string_lossy())));
        assert!(!entry_command_exists(&entry(
            &dir.path().join("gone").to_string_lossy()
        )));
        // A bare name is resolved through the client's `PATH`, not this
        // process's, so it is never called stale.
        assert!(entry_command_exists(&entry("heimdall")));
        assert!(entry_command_exists(&json!({ "args": [] })));
    }

    #[test]
    fn merging_preserves_every_other_server_and_setting() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(
            &path,
            r#"{"theme":"dark","mcpServers":{"other":{"command":"other-server"}}}"#,
        )
        .unwrap();

        // Exercise the merge directly; `install` resolves its own fixed paths.
        let existing = read_json(&path).unwrap();
        let key = server_key(&existing, "/v");
        let mut document = existing.as_object().unwrap().clone();
        let servers = document
            .entry("mcpServers".to_string())
            .or_insert_with(|| Value::Object(Map::new()));
        servers
            .as_object_mut()
            .unwrap()
            .insert(key, server_entry(Path::new("/x/heimdall"), "/v"));
        write_atomically(&path, &Value::Object(document)).unwrap();

        let written = read_json(&path).unwrap();
        assert_eq!(written["theme"], "dark");
        assert_eq!(written["mcpServers"]["other"]["command"], "other-server");
        assert_eq!(written["mcpServers"]["heimdall"]["args"][2], "/v");
    }

    #[test]
    fn a_write_leaves_no_temporary_file_behind() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        write_atomically(&path, &json!({ "a": 1 })).unwrap();

        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().to_string())
            .filter(|name| name.contains("tmp"))
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
    }

    #[test]
    fn an_existing_file_is_backed_up_before_it_is_changed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(&path, r#"{"theme":"dark"}"#).unwrap();

        let backup = back_up(&path).unwrap().expect("a backup path");
        assert_eq!(std::fs::read_to_string(backup).unwrap(), r#"{"theme":"dark"}"#);

        // Nothing to back up when the file does not exist yet.
        assert!(back_up(&dir.path().join("missing.json")).unwrap().is_none());
    }

    #[test]
    fn known_clients_are_reported_with_absolute_per_user_paths() {
        for client in known_clients("/v") {
            assert!(Path::new(&client.path).is_absolute(), "{}", client.path);
            assert!(!client.id.is_empty());
            assert!(!client.name.is_empty());
        }
    }
}

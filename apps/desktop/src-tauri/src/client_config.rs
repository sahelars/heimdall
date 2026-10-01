//! Writing the MCP server entry into known client configuration files.
//!
//! The Server screen offers this, and only ever with explicit consent
//! (SPEC §15). React names a client from the detected list — never a path — so
//! this can only touch files this module already knows about, and an existing
//! configuration is merged rather than replaced.
//!
//! Every client gets one entry, `heimdall`, running `heimdall mcp`: one server
//! for every vault the user shares, each call naming the vault it means.
//! Earlier builds wrote an entry per vault (`heimdall mcp --vault P`), which
//! multiplied with every vault and gave an agent several identical `read` and
//! `write` tools with nothing to tell them apart. Installing replaces those.

use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::{json, Map, Value};
use toml_edit::{Array, DocumentMut, Item, Table};

use crate::cli_bridge::{DomainError, SidecarOrigin};

/// How a client's configuration file is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Format {
    /// A JSON object with an `mcpServers` map, as Claude Desktop keeps it.
    Json,
    /// A TOML document with an `[mcp_servers.<name>]` table per server.
    Toml,
}

/// The table ChatGPT's Codex host reads its MCP servers from.
///
/// `~/.codex/config.toml` is shared by the ChatGPT desktop app, the Codex CLI
/// and its IDE extension; see OpenAI's Model Context Protocol documentation.
const TOML_SERVERS: &str = "mcp_servers";

struct Client {
    id: &'static str,
    name: &'static str,
    format: Format,
    /// The file, relative to the user's home directory.
    segments: &'static [&'static str],
}

/// The configuration files this application knows how to edit.
///
/// Each entry is a fixed, per-user location. Nothing here is derived from
/// anything React sends.
const KNOWN_CLIENTS: &[Client] = &[
    Client {
        id: "claude-desktop",
        name: "Claude Desktop",
        format: Format::Json,
        segments: &["Library", "Application Support", "Claude", "claude_desktop_config.json"],
    },
    Client {
        id: "chatgpt",
        name: "ChatGPT",
        format: Format::Toml,
        segments: &[".codex", "config.toml"],
    },
];

/// The one entry name Heimdall uses in every client.
pub const SERVER_KEY: &str = "heimdall";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KnownClient {
    pub id: String,
    pub name: String,
    pub path: String,
    /// Whether the configuration file exists today.
    pub present: bool,
    /// Whether it has the one `heimdall` entry, serving the shared vaults.
    pub installed: bool,
    /// Whether that entry names a command that is no longer there.
    ///
    /// A client whose configured command cannot be spawned reports a timeout,
    /// never a missing file, so this is the only place the user can find out.
    pub stale: bool,
    /// Older Heimdall entries, one per vault (`heimdall mcp --vault P`), that
    /// adding the entry replaces. Each one's vault is shared first, so nothing
    /// a client could reach before is lost.
    pub legacy: Vec<LegacyEntry>,
    /// Set when `heimdall` names something that is not Heimdall, which this
    /// application will not overwrite.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conflict: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyEntry {
    /// The entry's name in the client's configuration.
    pub key: String,
    /// The vault it served.
    pub vault: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallOutcome {
    pub path: String,
    pub server_key: String,
    /// Whether an existing `heimdall` entry was replaced.
    pub replaced: bool,
    /// The older per-vault entries removed in favour of the one entry.
    pub removed: Vec<String>,
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

/// Whether an entry runs some build of Heimdall's server.
///
/// Judged by what it runs, not what it is called: older Heimdall builds named
/// entries `heimdall-<folder>`, and a user may have named one anything.
fn is_heimdall(entry: &Value) -> bool {
    let command = entry.get("command").and_then(Value::as_str).unwrap_or_default();
    let runs_heimdall = Path::new(command)
        .file_name()
        .is_some_and(|name| name.to_string_lossy().starts_with("heimdall"));
    let first_arg = entry
        .get("args")
        .and_then(Value::as_array)
        .and_then(|args| args.first())
        .and_then(Value::as_str);
    runs_heimdall && first_arg == Some("mcp")
}

/// The vault an older, one-vault-per-entry Heimdall server was fixed to.
fn fixed_vault(entry: &Value) -> Option<String> {
    let args = entry.get("args").and_then(Value::as_array)?;
    let position = args.iter().position(|arg| arg.as_str() == Some("--vault"))?;
    args.get(position + 1).and_then(Value::as_str).map(str::to_string)
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

/// What a configuration holds of Heimdall's, in the JSON shape `servers_of`
/// produces.
struct Survey {
    /// The current `heimdall` entry, when it is the shared-vault server.
    current: Option<Value>,
    legacy: Vec<LegacyEntry>,
    conflict: Option<String>,
}

fn survey(existing: &Value) -> Survey {
    let servers = existing
        .get("mcpServers")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();

    let mut current = None;
    let mut legacy = Vec::new();
    let mut conflict = None;
    for (key, entry) in &servers {
        if !is_heimdall(entry) {
            if key == SERVER_KEY {
                conflict = Some(format!(
                    "\"{SERVER_KEY}\" in this file runs something other than Heimdall; rename \
                     or remove that entry first"
                ));
            }
            continue;
        }
        match fixed_vault(entry) {
            Some(vault) => legacy.push(LegacyEntry {
                key: key.clone(),
                vault,
            }),
            None if key == SERVER_KEY => current = Some(entry.clone()),
            // A shared-vault server under another name is a duplicate of the
            // one this writes, and goes with the legacy entries.
            None => legacy.push(LegacyEntry {
                key: key.clone(),
                vault: String::new(),
            }),
        }
    }
    Survey {
        current,
        legacy,
        conflict,
    }
}

/// The server entry a client should run: the shared vaults, by name.
pub fn server_entry(command: &Path) -> Value {
    json!({
        "command": command.to_string_lossy(),
        "args": ["mcp"],
    })
}

/// Which known clients are present, and what each holds of Heimdall's.
pub fn known_clients() -> Vec<KnownClient> {
    KNOWN_CLIENTS
        .iter()
        .filter_map(|client| {
            let path = config_path(client.segments)?;
            let existing = std::fs::read_to_string(&path)
                .ok()
                .and_then(|text| servers_of(client.format, &text))
                .unwrap_or_else(|| json!({}));
            let survey = survey(&existing);

            Some(KnownClient {
                id: client.id.to_string(),
                name: client.name.to_string(),
                path: path.to_string_lossy().to_string(),
                present: path.is_file(),
                installed: survey.current.is_some(),
                stale: survey.current.as_ref().is_some_and(|entry| !entry_command_exists(entry)),
                legacy: survey.legacy,
                conflict: survey.conflict,
            })
        })
        .collect()
}

/// The vaults older per-vault entries in one client served, so the caller
/// can share them before [`install`] removes those entries.
pub fn legacy_vaults(client_id: &str) -> Result<Vec<String>, DomainError> {
    let client = client(client_id)?;
    let Some(path) = config_path(client.segments) else {
        return Ok(Vec::new());
    };
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Ok(Vec::new());
    };
    let existing = servers_of(client.format, &text).unwrap_or_else(|| json!({}));
    Ok(survey(&existing)
        .legacy
        .into_iter()
        .map(|entry| entry.vault)
        .filter(|vault| !vault.is_empty())
        .collect())
}

/// A configuration's servers, in the JSON shape the survey reads
/// (`{"mcpServers": {name: {command, args}}}`) whatever the file's format.
///
/// `None` when the text does not parse.
fn servers_of(format: Format, text: &str) -> Option<Value> {
    match format {
        Format::Json => parse_json(text),
        Format::Toml => Some(toml_servers(&text.parse::<DocumentMut>().ok()?)),
    }
}

fn parse_json(text: &str) -> Option<Value> {
    if text.trim().is_empty() {
        return Some(json!({}));
    }
    serde_json::from_str(text).ok()
}

fn toml_servers(document: &DocumentMut) -> Value {
    let mut servers = Map::new();
    if let Some(table) = document.get(TOML_SERVERS).and_then(Item::as_table_like) {
        for (name, item) in table.iter() {
            let mut entry = Map::new();
            if let Some(fields) = item.as_table_like() {
                if let Some(command) = fields.get("command").and_then(Item::as_str) {
                    entry.insert("command".to_string(), json!(command));
                }
                if let Some(args) = fields.get("args").and_then(Item::as_array) {
                    let args = args.iter().filter_map(|arg| arg.as_str()).map(|arg| json!(arg));
                    entry.insert("args".to_string(), Value::Array(args.collect()));
                }
            }
            servers.insert(name.to_string(), Value::Object(entry));
        }
    }
    json!({ "mcpServers": servers })
}

/// What merging the server entry into a configuration produced.
struct Merged {
    replaced: bool,
    removed: Vec<String>,
    /// The whole file, ready to write.
    text: String,
}

/// Merge the server entry into a configuration's text, keeping everything
/// else and removing Heimdall's older per-vault entries.
///
/// `existing` is `None` when the file does not exist yet.
fn merge(format: Format, existing: Option<&str>, command: &Path) -> Result<Merged, DomainError> {
    // A file that exists but does not parse is someone's working
    // configuration. Overwriting it would destroy settings this application
    // knows nothing about, so it stops instead.
    let unreadable = |kind: &str| {
        DomainError::new(
            "INVALID_INPUT",
            format!("that configuration file is not valid {kind}; open it and fix it first"),
        )
    };
    let refuse_conflict = |survey: &Survey| match &survey.conflict {
        Some(message) => Err(DomainError::new("ALREADY_EXISTS", message.clone())),
        None => Ok(()),
    };

    match format {
        Format::Json => {
            let existing = match existing {
                Some(text) => parse_json(text).ok_or_else(|| unreadable("JSON"))?,
                None => json!({}),
            };
            let survey = survey(&existing);
            refuse_conflict(&survey)?;
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

            let replaced = servers.contains_key(SERVER_KEY);
            let removed: Vec<String> = survey.legacy.iter().map(|entry| entry.key.clone()).collect();
            for key in &removed {
                servers.remove(key);
            }
            servers.insert(SERVER_KEY.to_string(), server_entry(command));

            let mut text = serde_json::to_string_pretty(&Value::Object(document)).map_err(|_| {
                DomainError::new("INTERNAL_ERROR", "could not render the configuration")
            })?;
            text.push('\n');
            Ok(Merged { replaced, removed, text })
        }
        Format::Toml => {
            let mut document = existing
                .unwrap_or("")
                .parse::<DocumentMut>()
                .map_err(|_| unreadable("TOML"))?;
            let survey = survey(&toml_servers(&document));
            refuse_conflict(&survey)?;

            let servers = document.entry(TOML_SERVERS).or_insert_with(|| {
                // Implicit, so a new file reads `[mcp_servers.heimdall]` with
                // no empty `[mcp_servers]` header above it.
                let mut table = Table::new();
                table.set_implicit(true);
                Item::Table(table)
            });
            let Some(servers) = servers.as_table_like_mut() else {
                return Err(DomainError::new(
                    "INVALID_INPUT",
                    format!(
                        "that configuration file has a \"{TOML_SERVERS}\" value that is not a table"
                    ),
                ));
            };

            let replaced = servers.contains_key(SERVER_KEY);
            let removed: Vec<String> = survey.legacy.iter().map(|entry| entry.key.clone()).collect();
            for key in &removed {
                servers.remove(key);
            }
            let mut entry = Table::new();
            entry.insert("command", toml_edit::value(command.to_string_lossy().as_ref()));
            let args: Array = ["mcp"].into_iter().collect();
            entry.insert("args", toml_edit::value(args));
            servers.insert(SERVER_KEY, Item::Table(entry));

            Ok(Merged { replaced, removed, text: document.to_string() })
        }
    }
}

fn client(client_id: &str) -> Result<&'static Client, DomainError> {
    KNOWN_CLIENTS.iter().find(|client| client.id == client_id).ok_or_else(|| {
        DomainError::new(
            "INVALID_INPUT",
            format!("\"{client_id}\" is not a client this application can configure"),
        )
    })
}

/// Merge the server entry into one known client's configuration.
///
/// Callers share the vaults [`legacy_vaults`] names first: the older entries
/// this removes are the only record that a client could reach them.
pub fn install(
    client_id: &str,
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

    let client = client(client_id)?;
    let path = config_path(client.segments).ok_or_else(|| {
        DomainError::new("IO_ERROR", "could not locate this user's home directory")
    })?;

    let existing = if path.is_file() {
        Some(std::fs::read_to_string(&path).map_err(|err| {
            DomainError::new(
                "IO_ERROR",
                format!("could not read the existing configuration: {}", err.kind()),
            )
        })?)
    } else {
        None
    };
    let merged = merge(client.format, existing.as_deref(), command)?;

    let backup_path = back_up(&path)?;
    write_atomically(&path, &merged.text)?;

    Ok(InstallOutcome {
        path: path.to_string_lossy().to_string(),
        server_key: SERVER_KEY.to_string(),
        replaced: merged.replaced,
        removed: merged.removed,
        backup_path,
    })
}

/// Keep a copy of what was there before touching a file the user owns.
fn back_up(path: &Path) -> Result<Option<String>, DomainError> {
    if !path.is_file() {
        return Ok(None);
    }
    // Keeps the file's own extension, so the copy still opens as what it is.
    let extension = path.extension().map(|ext| ext.to_string_lossy()).unwrap_or_default();
    let backup = path.with_extension(format!("heimdall-backup.{extension}"));
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
fn write_atomically(path: &Path, text: &str) -> Result<(), DomainError> {
    let parent = path.parent().ok_or_else(|| {
        DomainError::new("IO_ERROR", "that configuration path has no parent directory")
    })?;
    std::fs::create_dir_all(parent).map_err(|err| {
        DomainError::new(
            "IO_ERROR",
            format!("could not create the configuration directory: {}", err.kind()),
        )
    })?;

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

    fn legacy(key: &str, command: &str, vault: &str) -> (String, Value) {
        (key.to_string(), json!({ "command": command, "args": ["mcp", "--vault", vault] }))
    }

    fn servers(entries: Vec<(String, Value)>) -> Value {
        json!({ "mcpServers": entries.into_iter().collect::<Map<String, Value>>() })
    }

    #[test]
    fn the_entry_runs_the_bundled_command_for_the_shared_vaults() {
        let entry = server_entry(Path::new("/Applications/Heimdall.app/Contents/MacOS/heimdall"));
        assert_eq!(entry["command"], "/Applications/Heimdall.app/Contents/MacOS/heimdall");
        assert_eq!(entry["args"], json!(["mcp"]));
    }

    #[test]
    fn heimdall_entries_are_recognised_by_what_they_run_not_their_name() {
        let found = survey(&servers(vec![
            legacy("heimdall", "/x/target/debug/heimdall", "/Users/n/Vault-test"),
            legacy("heimdall-heimdall-vault", "/Applications/Heimdall.app/Contents/MacOS/heimdall", "/Users/n/Heimdall-Vault"),
            legacy("notes", "/opt/heimdall", "/Users/n/Notes"),
            ("other".to_string(), json!({ "command": "other-server", "args": ["mcp", "--vault", "/x"] })),
        ]));
        let keys: Vec<_> = found.legacy.iter().map(|entry| entry.key.as_str()).collect();
        assert_eq!(keys, ["heimdall", "heimdall-heimdall-vault", "notes"]);
        assert_eq!(found.legacy[1].vault, "/Users/n/Heimdall-Vault");
        assert!(found.current.is_none());
        assert!(found.conflict.is_none());
    }

    #[test]
    fn installing_replaces_every_per_vault_entry_with_one() {
        let existing = serde_json::to_string(&json!({
            "theme": "dark",
            "mcpServers": {
                "heimdall": { "command": "/x/heimdall", "args": ["mcp", "--vault", "/Users/n/Personal"] },
                "heimdall-work-notes": { "command": "/x/heimdall", "args": ["mcp", "--vault", "/Users/n/Work Notes"] },
                "other": { "command": "other-server" }
            }
        }))
        .unwrap();
        let merged = merge(Format::Json, Some(&existing), Path::new("/new/heimdall")).unwrap();
        assert!(merged.replaced);
        assert_eq!(merged.removed, ["heimdall", "heimdall-work-notes"]);

        let written = parse_json(&merged.text).unwrap();
        assert_eq!(written["theme"], "dark");
        assert_eq!(written["mcpServers"]["other"]["command"], "other-server");
        let names: Vec<_> = written["mcpServers"].as_object().unwrap().keys().cloned().collect();
        assert_eq!(names, ["heimdall", "other"]);
        assert_eq!(written["mcpServers"]["heimdall"], server_entry(Path::new("/new/heimdall")));
    }

    #[test]
    fn reinstalling_is_one_entry_still() {
        let first = merge(Format::Json, None, Path::new("/old/heimdall")).unwrap();
        assert!(!first.replaced);
        let second = merge(Format::Json, Some(&first.text), Path::new("/new/heimdall")).unwrap();
        assert!(second.replaced);
        assert!(second.removed.is_empty());
        let survey = survey(&parse_json(&second.text).unwrap());
        assert_eq!(survey.current.unwrap()["command"], "/new/heimdall");
    }

    #[test]
    fn a_heimdall_key_that_belongs_to_something_else_is_left_alone() {
        let existing = r#"{"mcpServers":{"heimdall":{"command":"/usr/local/bin/other","args":[]}}}"#;
        let error = merge(Format::Json, Some(existing), Path::new("/x/heimdall"))
            .err()
            .expect("a refusal");
        assert_eq!(error.code, "ALREADY_EXISTS");
        assert!(error.message.contains("other than Heimdall"), "{}", error.message);

        let toml = "[mcp_servers.heimdall]\ncommand = \"other\"\nargs = []\n";
        assert!(merge(Format::Toml, Some(toml), Path::new("/x/heimdall")).is_err());
    }

    #[test]
    fn only_known_clients_can_be_written_to() {
        let shipped = SidecarOrigin::Shipped;
        let error = install("/etc/passwd", Path::new("/x/heimdall"), shipped).unwrap_err();
        assert_eq!(error.code, "INVALID_INPUT");

        let error = install("../../evil", Path::new("/x/heimdall"), shipped).unwrap_err();
        assert_eq!(error.code, "INVALID_INPUT");
        assert!(legacy_vaults("../../evil").is_err());
    }

    #[test]
    fn a_development_build_is_refused_before_anything_is_touched() {
        // The path a development build would write disappears on a rebuild, and
        // the client it was written for then reports a timeout rather than a
        // missing file — so the refusal has to explain itself (SPEC §15).
        let backup = config_path(KNOWN_CLIENTS[0].segments)
            .expect("a home directory")
            .with_extension("heimdall-backup.json");
        let before = backup.is_file();

        let error = install(
            KNOWN_CLIENTS[0].id,
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

        let entry = |command: &str| json!({ "command": command, "args": ["mcp"] });

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
    fn a_write_leaves_no_temporary_file_behind() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        write_atomically(&path, "{ \"a\": 1 }\n").unwrap();

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
    fn a_toml_merge_keeps_every_other_server_setting_and_comment() {
        let existing = r#"# my settings
model = "gpt-5"

[mcp_servers.other]
command = "other-server" # keep me
args = []
"#;
        let merged = merge(Format::Toml, Some(existing), Path::new("/x/heimdall")).unwrap();
        assert!(!merged.replaced);

        assert!(merged.text.starts_with(existing), "{}", merged.text);
        let document: DocumentMut = merged.text.parse().unwrap();
        assert_eq!(document["model"].as_str(), Some("gpt-5"));
        assert_eq!(document["mcp_servers"]["heimdall"]["command"].as_str(), Some("/x/heimdall"));
        let servers = toml_servers(&document);
        assert_eq!(servers["mcpServers"]["heimdall"]["args"], json!(["mcp"]));
        assert_eq!(servers["mcpServers"]["other"]["command"], "other-server");
    }

    #[test]
    fn a_new_toml_file_gets_only_the_server_table() {
        let merged = merge(Format::Toml, None, Path::new("/x/heimdall")).unwrap();
        assert_eq!(
            merged.text,
            "[mcp_servers.heimdall]\ncommand = \"/x/heimdall\"\nargs = [\"mcp\"]\n"
        );
    }

    #[test]
    fn a_toml_migration_removes_only_heimdalls_per_vault_tables() {
        // The shape this application wrote before, beside a server of
        // ChatGPT's own that must survive untouched.
        let existing = r#"[mcp_servers.node_repl]
command = "/Applications/ChatGPT.app/Contents/Resources/cua_node/bin/node_repl"
args = []

[mcp_servers.heimdall]
command = "/x/target/debug/heimdall"
args = ["mcp", "--vault", "/Users/n/Vault-test"]

[mcp_servers.heimdall-heimdall-vault]
command = "/Applications/Heimdall.app/Contents/MacOS/heimdall"
args = ["mcp", "--vault", "/Users/n/Heimdall-Vault"]
"#;
        let merged = merge(Format::Toml, Some(existing), Path::new("/Applications/Heimdall.app/Contents/MacOS/heimdall")).unwrap();
        assert_eq!(merged.removed, ["heimdall", "heimdall-heimdall-vault"]);
        let servers = servers_of(Format::Toml, &merged.text).unwrap();
        let names: Vec<_> = servers["mcpServers"].as_object().unwrap().keys().cloned().collect();
        assert_eq!(names, ["heimdall", "node_repl"]);
        assert_eq!(servers["mcpServers"]["heimdall"]["args"], json!(["mcp"]));
        assert!(merged.text.contains("node_repl"));
    }

    #[test]
    fn a_configuration_that_does_not_parse_is_refused() {
        let error = merge(Format::Toml, Some("[mcp_servers"), Path::new("/x/heimdall"))
            .err()
            .expect("a refusal");
        assert_eq!(error.code, "INVALID_INPUT");
        assert!(error.message.contains("TOML"), "{}", error.message);

        let error = merge(Format::Toml, Some("mcp_servers = 3\n"), Path::new("/x/heimdall"))
            .err()
            .expect("a refusal");
        assert_eq!(error.code, "INVALID_INPUT");

        let error = merge(Format::Json, Some("{"), Path::new("/x/heimdall"))
            .err()
            .expect("a refusal");
        assert!(error.message.contains("JSON"), "{}", error.message);
    }

    #[test]
    fn a_backup_keeps_the_files_own_extension() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "model = \"x\"\n").unwrap();
        let backup = back_up(&path).unwrap().expect("a backup path");
        assert!(backup.ends_with("config.heimdall-backup.toml"), "{backup}");
    }

    #[test]
    fn the_clients_offered_are_claude_desktop_and_chatgpt() {
        let ids: Vec<_> = KNOWN_CLIENTS.iter().map(|client| client.id).collect();
        assert_eq!(ids, ["claude-desktop", "chatgpt"]);
    }

    #[test]
    fn known_clients_are_reported_with_absolute_per_user_paths() {
        for client in known_clients() {
            assert!(Path::new(&client.path).is_absolute(), "{}", client.path);
            assert!(!client.id.is_empty());
            assert!(!client.name.is_empty());
        }
    }
}

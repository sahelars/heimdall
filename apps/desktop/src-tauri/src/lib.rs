//! The Tauri layer: launch the window and expose a small, typed command surface.
//!
//! Nothing here implements domain behavior. Each command validates its input,
//! delegates to the bundled CLI, and returns typed data (SPEC §4, §15).

// Public so the integration tests can drive the real bundled sidecar rather
// than a stand-in (SPEC §17).
pub mod cli_bridge;
pub mod client_config;
mod menu;

use serde_json::Value;
use tauri::Emitter;

use cli_bridge::{CliResponse, CliStatus, DomainError, HealthCheck};
use client_config::{InstallOutcome, KnownClient};

/// Which CLI this application is using, and what it reports.
#[tauri::command]
async fn cli_status() -> CliStatus {
    blocking(cli_bridge::status).await
}

/// Run one allowlisted CLI command.
///
/// `command` is a subcommand name checked against a fixed list, `request` is a
/// flat object whose keys are checked against that command's own arguments, and
/// `stdin` carries Markdown. React cannot name an executable or pass a raw
/// command string.
#[tauri::command]
async fn invoke_cli(command: String, request: Value, stdin: Option<String>) -> CliResponse {
    blocking(move || cli_bridge::run(&command, &request, stdin.as_deref())).await
}

/// Launch the MCP server for the shared vaults and complete a real handshake.
#[tauri::command]
async fn health_check() -> HealthCheck {
    blocking(cli_bridge::health_check).await
}

/// The MCP client configuration files this application can write to.
#[tauri::command]
async fn list_client_configs() -> Vec<KnownClient> {
    blocking(client_config::known_clients).await
}

/// Write the one server entry into a known client's configuration.
///
/// Only reachable for a client named in [`list_client_configs`], and only ever
/// called after the user asks for it (SPEC §15). The vaults its older
/// per-vault entries served are shared first; if any of those cannot be
/// shared, the configuration is left exactly as it was.
#[tauri::command]
async fn install_client_config(client_id: String) -> Result<InstallOutcome, DomainError> {
    blocking(move || {
        let command = cli_bridge::sidecar_path();
        if !command.is_file() {
            return Err(DomainError::new(
                "IO_ERROR",
                "the bundled heimdall command line tool is missing, so there is nothing to point a client at",
            ));
        }
        let origin = cli_bridge::sidecar_origin();
        if origin.is_development() {
            // Refused before sharing anything, for the reason `install` gives.
            return client_config::install(&client_id, &command, origin);
        }
        for vault in client_config::legacy_vaults(&client_id)? {
            // A folder that is gone has nothing left to share.
            if !std::path::Path::new(&vault).is_dir() {
                continue;
            }
            let shared = cli_bridge::run("share", &serde_json::json!({ "vault": vault }), None);
            if let Some(error) = shared.error {
                return Err(error);
            }
        }
        client_config::install(&client_id, &command, origin)
    })
    .await
}

/// System Settings, opened at Privacy & Security › Full Disk Access.
///
/// An AI client launches `heimdall mcp` itself, and macOS judges that process
/// on its own account; for a vault in Documents, Desktop, Downloads, or iCloud
/// Drive it is refused without a prompt (SPEC §16). Adding the bundled CLI
/// here is the fix, so the Server screen takes the user straight to it.
pub const PRIVACY_SETTINGS_URL: &str =
    "x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles";

/// The one program both commands below run, by absolute path: no `PATH`
/// lookup and no shell.
const OPEN: &str = "/usr/bin/open";

/// Open System Settings at Full Disk Access. Takes nothing from React.
#[tauri::command]
async fn open_privacy_settings() -> Result<(), DomainError> {
    blocking(|| launch(&[PRIVACY_SETTINGS_URL])).await
}

/// Select the bundled CLI in Finder, so it can be dragged into Full Disk
/// Access. Takes nothing from React: the path is the sidecar this app runs.
#[tauri::command]
async fn reveal_sidecar() -> Result<(), DomainError> {
    blocking(|| {
        let sidecar = cli_bridge::sidecar_path();
        if !sidecar.is_file() {
            return Err(DomainError::new(
                "IO_ERROR",
                "the bundled heimdall command line tool is missing from this application",
            ));
        }
        launch(&["-R".as_ref(), sidecar.as_os_str()])
    })
    .await
}

fn launch<S: AsRef<std::ffi::OsStr>>(args: &[S]) -> Result<(), DomainError> {
    std::process::Command::new(OPEN)
        .args(args)
        .status()
        .map_err(|err| DomainError::new("IO_ERROR", format!("could not open it: {}", err.kind())))
        .and_then(|status| {
            if status.success() {
                Ok(())
            } else {
                Err(DomainError::new("IO_ERROR", "macOS could not open it"))
            }
        })
}

/// Run blocking work off the UI thread.
///
/// Every command here spawns a process or touches the filesystem, and the
/// window must stay responsive while it does.
async fn blocking<T, F>(work: F) -> T
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    tauri::async_runtime::spawn_blocking(work)
        .await
        .expect("bridge task")
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .menu(menu::build)
        .on_menu_event(|app, event| {
            if event.id() == menu::SETTINGS_ITEM {
                // One window, one modal. Emitting rather than opening a second
                // window keeps a single view of the vault.
                let _ = app.emit(menu::SETTINGS_EVENT, ());
            }
        })
        .invoke_handler(tauri::generate_handler![
            cli_status,
            invoke_cli,
            health_check,
            list_client_configs,
            install_client_config,
            open_privacy_settings,
            reveal_sidecar,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Heimdall");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_privacy_link_is_fixed_and_points_at_full_disk_access() {
        // Nothing React sends reaches `open`: the URL is a constant, and both
        // commands take no arguments at all.
        assert!(PRIVACY_SETTINGS_URL.starts_with("x-apple.systempreferences:"));
        assert!(PRIVACY_SETTINGS_URL.ends_with("Privacy_AllFiles"));
        assert_eq!(OPEN, "/usr/bin/open");
    }
}

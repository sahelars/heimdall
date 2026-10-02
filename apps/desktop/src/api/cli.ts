/**
 * The only way this application reaches the domain.
 *
 * Every call goes through a Tauri command into Rust, which runs the bundled
 * CLI. React never names an executable, builds a command string, or touches the
 * filesystem (SPEC §15).
 */

import { invoke } from "@tauri-apps/api/core";

import type {
  CliResponse,
  CliStatus,
  CreateVaultData,
  HealthCheck,
  InstallOutcome,
  KnownClient,
  ReadData,
  VaultsData,
} from "./types";

/** Which CLI is in use, and what it reports about itself. */
export function cliStatus(): Promise<CliStatus> {
  return invoke<CliStatus>("cli_status");
}

/**
 * Run one allowlisted CLI subcommand.
 *
 * `request` keys are checked in Rust against that subcommand's own arguments,
 * so an unknown key is refused rather than passed along.
 */
export function invokeCli<T>(
  command: string,
  request: Record<string, unknown> = {},
  stdin?: string,
): Promise<CliResponse<T>> {
  return invoke<CliResponse<T>>("invoke_cli", { command, request, stdin });
}

/**
 * Create a vault: scaffold an empty or missing folder, or register an existing
 * folder of notes as one without writing anything into it.
 */
export function createVault(name: string, root: string) {
  return invokeCli<CreateVaultData>("create", { name, root });
}

/**
 * Confirm a folder is a readable vault by listing its root.
 *
 * A folder that has moved or cannot be read answers with a domain error, which
 * is exactly what the Setup screen needs to show.
 */
export function inspectVault(vault: string) {
  return invokeCli<ReadData>("read", { vault });
}

/** Launch the MCP server for the shared vaults and complete a real handshake. */
export function healthCheck(): Promise<HealthCheck> {
  return invoke<HealthCheck>("health_check");
}

/** Which MCP client configurations this application can write to. */
export function listClientConfigs(): Promise<KnownClient[]> {
  return invoke<KnownClient[]>("list_client_configs");
}

/**
 * Write the one server entry into a known client's configuration, replacing
 * any per-vault entries an older build wrote (their vaults are shared first).
 */
export function installClientConfig(clientId: string): Promise<InstallOutcome> {
  return invoke<InstallOutcome>("install_client_config", { clientId });
}

/** Every vault Heimdall knows, and which are shared with AI clients. */
export function listVaults() {
  return invokeCli<VaultsData>("vaults");
}

/** Share a vault with AI clients — or rename it, if it already is. */
export function shareVault(vault: string, name?: string) {
  return invokeCli<{ name: string; path: string; shared: true }>(
    "share",
    name ? { vault, name } : { vault },
  );
}

/** Stop sharing a vault with AI clients. */
export function unshareVault(vault: string) {
  return invokeCli<{ path: string; shared: false; changed: boolean }>("unshare", { vault });
}

/** Open System Settings at Privacy & Security › Full Disk Access. */
export function openPrivacySettings(): Promise<void> {
  return invoke<void>("open_privacy_settings");
}

/** Select the bundled command line tool in Finder, to add it to Full Disk Access. */
export function revealCli(): Promise<void> {
  return invoke<void>("reveal_sidecar");
}

/** The exact command a client runs: one server for every shared vault. */
export function mcpCommand(cliPath: string): string {
  return `${quote(cliPath)} mcp`;
}

/**
 * Where macOS privacy protection keeps an app launched by another app out
 * unless the user allows it: Documents, Desktop, Downloads, iCloud Drive,
 * other cloud storage, and external or network volumes. Returns a label for
 * the location, or null.
 *
 * Heimdall itself may have access there while an AI client's copy of the
 * server does not, which is exactly the case a health check run from here
 * cannot see (SPEC §15).
 */
export function protectedLocation(vault: string): string | null {
  const home = /^\/Users\/[^/]+\/(Documents|Desktop|Downloads|Library\/Mobile Documents|Library\/CloudStorage)(\/|$)/.exec(vault);
  if (home) {
    const place = home[1]!;
    if (place === "Library/Mobile Documents") return "iCloud Drive";
    if (place === "Library/CloudStorage") return "cloud storage";
    return place;
  }
  if (/^\/Volumes\//.test(vault)) return "an external or network volume";
  return null;
}

/**
 * The `mcpServers` snippet to paste into a client's configuration.
 *
 * One entry, whatever the number of vaults: the server serves every shared
 * vault and each call names one. The command is the bundled sidecar's absolute
 * path, so a client launches the same binary this application uses rather than
 * whatever `heimdall` happens to be on its `PATH` (SPEC §16).
 */
export function clientConfigSnippet(cliPath: string): string {
  return JSON.stringify({ mcpServers: { heimdall: { command: cliPath, args: ["mcp"] } } }, null, 2);
}

/**
 * The `[mcp_servers.heimdall]` table to paste into ChatGPT's `~/.codex/config.toml`.
 *
 * Exactly what "Add entry" writes into a file that has none yet, so pasting by
 * hand and installing produce the same configuration. A JSON string is also a
 * valid TOML basic string.
 */
export function chatgptConfigSnippet(cliPath: string): string {
  return `[mcp_servers.heimdall]\ncommand = ${JSON.stringify(cliPath)}\nargs = ["mcp"]\n`;
}

/** Quote only when a shell would otherwise split the value. */
function quote(value: string): string {
  return /[\s"'\\$`;&|<>()]/.test(value) ? `"${value.replace(/(["\\$`])/g, "\\$1")}"` : value;
}

/** The last path segment, for labelling a vault in the UI. */
export function vaultName(vault: string): string {
  const parts = vault.replace(/[/\\]+$/, "").split(/[/\\]/);
  return parts[parts.length - 1] ?? vault;
}

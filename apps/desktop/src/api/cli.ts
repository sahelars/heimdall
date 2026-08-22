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
  ListMemoriesData,
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

/** Create a vault, or add the managed structure to an existing folder. */
export function createVault(name: string, root: string) {
  return invokeCli<CreateVaultData>("create", { name, root });
}

/**
 * Confirm a folder is a usable vault by reading its memories.
 *
 * An uninitialized vault answers `NOT_INITIALIZED` with guidance, which is
 * exactly what the Setup screen needs to show.
 */
export function inspectVault(vault: string) {
  return invokeCli<ListMemoriesData>("list-memories", { vault });
}

/** Launch the MCP server and complete a real handshake against it. */
export function healthCheck(vault: string): Promise<HealthCheck> {
  return invoke<HealthCheck>("health_check", { vault });
}

/** Which MCP client configurations this application can write to. */
export function listClientConfigs(vault: string): Promise<KnownClient[]> {
  return invoke<KnownClient[]>("list_client_configs", { vault });
}

/** Write the server entry into one known client's configuration. */
export function installClientConfig(
  clientId: string,
  vault: string,
): Promise<InstallOutcome> {
  return invoke<InstallOutcome>("install_client_config", { clientId, vault });
}

/** The exact command a user would run themselves. */
export function mcpCommand(cliPath: string, vault: string): string {
  return `${quote(cliPath)} mcp --vault ${quote(vault)}`;
}

/**
 * The `mcpServers` snippet to paste into a client's configuration.
 *
 * The command is the bundled sidecar's absolute path, so a client launches the
 * same binary this application uses rather than whatever `heimdall` happens to
 * be on its `PATH` (SPEC §16).
 */
export function clientConfigSnippet(
  cliPath: string,
  vault: string,
  serverKey = "heimdall",
): string {
  return JSON.stringify(
    { mcpServers: { [serverKey]: { command: cliPath, args: ["mcp", "--vault", vault] } } },
    null,
    2,
  );
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

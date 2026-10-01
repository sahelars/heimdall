/**
 * Server — make the local MCP server available to AI clients (SPEC §15).
 *
 * One server serves every vault the user shares, and each tool call names the
 * vault it means. So this screen does three things: chooses which vaults AI
 * clients may see and what they are called, puts the one `heimdall` entry into
 * a client's configuration (replacing the per-vault entries older builds
 * wrote), and proves the server answers with a real handshake.
 */

import { useCallback, useEffect, useState } from "react";

import {
  chatgptConfigSnippet,
  clientConfigSnippet,
  healthCheck,
  installClientConfig,
  listClientConfigs,
  listVaults,
  mcpCommand,
  protectedLocation,
  shareVault,
  unshareVault,
} from "../../api/cli";
import type {
  CliResponse,
  CliStatus,
  DomainError,
  HealthCheck,
  InstallOutcome,
  KnownClient,
  KnownVault,
} from "../../api/types";
import { Button, Facts, Failure, Notice, Panel } from "../../components";
import { Prompt } from "../../components/Prompt";

/** What each client is called here, and what a user needs to know about it. */
const CLIENT_NOTES: Record<string, string> = {
  chatgpt:
    "Work or Codex mode only — ChatGPT's plain chat and the web do not run local servers. Restart ChatGPT after adding.",
  "claude-desktop": "Restart Claude Desktop after adding.",
};

export function Server({ vault, status }: { vault: string; status: CliStatus | null }) {
  const [clients, setClients] = useState<KnownClient[]>([]);
  const [vaults, setVaults] = useState<KnownVault[]>([]);
  const [installed, setInstalled] = useState<InstallOutcome | null>(null);
  const [health, setHealth] = useState<HealthCheck | null>(null);
  const [error, setError] = useState<DomainError | null>(null);
  const [busy, setBusy] = useState(false);
  const [renaming, setRenaming] = useState<KnownVault | null>(null);

  const cliPath = status?.path ?? "";
  // A build directory's path is not one to leave in a file the user keeps: a
  // rebuild removes it, and the client then reports a timeout rather than a
  // missing file. The write is refused in Rust; this is so the reason arrives
  // before the click, and covers copying the snippet by hand too (SPEC §15).
  const developmentBuild = status?.developmentBuild ?? false;

  const refreshVaults = useCallback(async () => {
    try {
      const response = await listVaults();
      setVaults(response.ok ? (response.data?.vaults ?? []) : []);
    } catch {
      setVaults([]);
    }
  }, []);

  useEffect(() => {
    listClientConfigs().then(setClients).catch(() => setClients([]));
  }, [installed]);

  useEffect(() => {
    void refreshVaults();
  }, [refreshVaults, vault, installed]);

  /** Run one sharing command, then show the list as it now stands. */
  async function change(run: () => Promise<CliResponse<unknown>>) {
    setBusy(true);
    setError(null);
    try {
      const response = await run();
      if (!response.ok && response.error) setError(response.error);
      await refreshVaults();
    } finally {
      setBusy(false);
    }
  }

  async function check() {
    setBusy(true);
    setError(null);
    try {
      setHealth(await healthCheck());
    } finally {
      setBusy(false);
    }
  }

  async function install(clientId: string) {
    setBusy(true);
    setError(null);
    try {
      setInstalled(await installClientConfig(clientId));
    } catch (failure) {
      setError(failure as DomainError);
    } finally {
      setBusy(false);
    }
  }

  // The current vault is offered even before anything has registered it.
  const listed = vaults.some((known) => known.path === vault);
  const rows: KnownVault[] =
    vault && !listed
      ? [
          ...vaults,
          {
            path: vault,
            folder: vault.split("/").pop() ?? vault,
            exists: true,
            shared: false,
            name: null,
          },
        ]
      : vaults;
  const shared = rows.filter((known) => known.shared);
  const guarded = shared
    .map((known) => ({ known, place: protectedLocation(known.path) }))
    .filter((entry) => entry.place !== null);

  return (
    <>
      <Panel
        title="Vaults shared with AI clients"
        description="One server serves every vault shared here. A client names a vault on each call, by the name shown; it never sees where the vault is. Locks hold whichever vault a call names."
      >
        {rows.length === 0 ? (
          <p className="muted">No vaults yet. Create or choose one on the Vault screen.</p>
        ) : (
          <div className="stack">
            {rows.map((known) => (
              <div className="row" key={known.path}>
                <div className="field field--grow">
                  <span className="field__label">
                    {known.shared ? known.name : known.folder}
                    {known.shared ? " — shared" : " — not shared"}
                    {known.exists ? "" : " (folder missing)"}
                  </span>
                  <span className="field__hint">{known.path}</span>
                </div>
                {known.shared ? (
                  <>
                    <Button onClick={() => setRenaming(known)} disabled={busy}>
                      Rename
                    </Button>
                    <Button onClick={() => change(() => unshareVault(known.path))} disabled={busy}>
                      Stop sharing
                    </Button>
                  </>
                ) : (
                  <Button
                    onClick={() => change(() => shareVault(known.path))}
                    disabled={busy || !known.exists}
                  >
                    Share
                  </Button>
                )}
              </div>
            ))}
          </div>
        )}
        {guarded.length > 0 ? (
          <Notice title="macOS may keep AI clients out">
            {guarded.map(({ known, place }) => (
              <p className="muted" key={known.path}>
                “{known.name}” is in {place}. macOS asks before letting another app's copy of
                Heimdall open it; if a client reports “Operation not permitted”, allow Heimdall in
                System Settings › Privacy &amp; Security › Files and Folders (or Full Disk Access),
                or keep the vault somewhere else, such as a folder in your home directory.
              </p>
            ))}
          </Notice>
        ) : null}
      </Panel>

      <Prompt
        title="Rename shared vault"
        label="Name AI clients use"
        initial={renaming?.name ?? ""}
        note="Clients connected now see the new name on their next call."
        open={renaming !== null}
        onCancel={() => setRenaming(null)}
        onSubmit={(name) => {
          const target = renaming;
          setRenaming(null);
          if (target && name.trim() && name !== target.name) {
            void change(() => shareVault(target.path, name.trim()));
          }
        }}
      />

      <Panel
        title="Run the server"
        description="The command every client runs. It serves the vaults shared above, and no tool call can reach anything else."
      >
        <pre className="snippet">{mcpCommand(cliPath)}</pre>
      </Panel>

      <Panel
        title="Client configuration"
        description="Paste the entry for your client into its MCP configuration — one entry, however many vaults you share. It points at the copy of the command line tool bundled with this application."
      >
        <div className="field">
          <span className="field__label">Claude Desktop</span>
          <span className="field__hint">claude_desktop_config.json</span>
        </div>
        <pre className="snippet">{clientConfigSnippet(cliPath)}</pre>
        <div className="field">
          <span className="field__label">ChatGPT (Work or Codex mode)</span>
          <span className="field__hint">~/.codex/config.toml</span>
        </div>
        <pre className="snippet">{chatgptConfigSnippet(cliPath)}</pre>
        {developmentBuild ? (
          <Notice title="Development build">
            <p className="muted">
              This command is a build artifact. A rebuild or <code>cargo clean</code> removes it,
              and a client whose command has gone reports only a timeout. Use a built copy of the
              application for anything you intend to keep.
            </p>
          </Notice>
        ) : null}
      </Panel>

      <Panel
        title="Install automatically"
        description="Writes the entry above into a client's configuration, in that client's own format. Existing servers and settings are kept, per-vault entries from older versions are replaced (their vaults are shared first), and the previous file is backed up."
      >
        {developmentBuild ? (
          <Notice title="Not available in a development build">
            <p className="muted">
              Writing this build's command into a client's configuration would leave an entry that
              breaks on the next rebuild. Install the built application and add the entry from
              there.
            </p>
          </Notice>
        ) : null}
        {clients.length === 0 ? (
          <p className="muted">No known MCP clients were found on this machine.</p>
        ) : (
          <div className="stack">
            {clients.map((client) => (
              <div className="row" key={client.id}>
                <div className="field field--grow">
                  <span className="field__label">
                    {client.name}
                    {client.stale
                      ? " — configured, but its command is gone"
                      : client.installed && client.legacy.length === 0
                        ? " — already configured"
                        : client.present
                          ? ""
                          : " — not installed"}
                  </span>
                  <span className="field__hint">{client.path}</span>
                  {CLIENT_NOTES[client.id] ? (
                    <span className="field__hint">{CLIENT_NOTES[client.id]}</span>
                  ) : null}
                  {client.legacy.length > 0 ? (
                    <span className="field__hint">
                      Replaces {client.legacy.length} older per-vault{" "}
                      {client.legacy.length === 1 ? "entry" : "entries"}:{" "}
                      {client.legacy.map((entry) => entry.key).join(", ")}
                    </span>
                  ) : null}
                  {client.conflict ? (
                    <span className="field__hint">{client.conflict}</span>
                  ) : null}
                </div>
                <Button
                  onClick={() => install(client.id)}
                  disabled={busy || developmentBuild || Boolean(client.conflict)}
                >
                  {client.installed || client.legacy.length > 0 ? "Update entry" : "Add entry"}
                </Button>
              </div>
            ))}
          </div>
        )}
      </Panel>

      {installed ? (
        <Notice title="Configuration written">
          <Facts
            rows={[
              ["File", installed.path],
              ["Entry", installed.serverKey],
              ["Existing entry", installed.replaced ? "replaced" : "added"],
              ...(installed.removed.length > 0
                ? ([["Removed", installed.removed.join(", ")]] as [string, string][])
                : []),
              ...(installed.backupPath
                ? ([["Backup", installed.backupPath]] as [string, string][])
                : []),
            ]}
          />
        </Notice>
      ) : null}

      <Panel
        title="Health check"
        description="Starts the server and completes a real MCP handshake against it — the same thing your client does. It runs with Heimdall's own macOS permissions, so it cannot show whether macOS lets another app's copy into a protected folder."
        action={
          <Button primary onClick={check} disabled={busy}>
            {busy ? "Checking…" : "Run health check"}
          </Button>
        }
      >
        {health?.ok ? (
          <Notice title="Handshake succeeded">
            <Facts
              rows={[
                ["Server", `${health.serverName ?? "?"} ${health.serverVersion ?? ""}`.trim()],
                ["Protocol", health.protocolVersion ?? "unknown"],
                [
                  "Vaults shared",
                  shared.length === 0
                    ? "none"
                    : shared.map((known) => known.name ?? known.folder).join(", "),
                ],
              ]}
            />
          </Notice>
        ) : null}
        {health && !health.ok && health.error ? (
          <Failure error={health.error} stderr={health.stderr} />
        ) : null}
        {!health ? <p className="muted">Not run yet.</p> : null}
      </Panel>

      {error ? <Failure error={error} /> : null}
    </>
  );
}

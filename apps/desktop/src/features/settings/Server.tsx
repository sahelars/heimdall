/**
 * Server — make the local MCP server available to AI clients (SPEC §15).
 *
 * Shows the exact command, generates the `mcpServers` snippet, offers to write
 * it into a detected client's configuration, and proves the whole thing works
 * with a real handshake.
 */

import { useEffect, useState } from "react";

import {
  clientConfigSnippet,
  healthCheck,
  installClientConfig,
  listClientConfigs,
  mcpCommand,
} from "../../api/cli";
import type { CliStatus, DomainError, HealthCheck, InstallOutcome, KnownClient } from "../../api/types";
import { Button, Facts, Failure, Notice, Panel } from "../../components";

export function Server({ vault, status }: { vault: string; status: CliStatus | null }) {
  const [clients, setClients] = useState<KnownClient[]>([]);
  const [installed, setInstalled] = useState<InstallOutcome | null>(null);
  const [health, setHealth] = useState<HealthCheck | null>(null);
  const [error, setError] = useState<DomainError | null>(null);
  const [busy, setBusy] = useState(false);

  const cliPath = status?.path ?? "";
  const serverKey = clients.find((client) => client.present)?.serverKey ?? "heimdall";
  // A build directory's path is not one to leave in a file the user keeps: a
  // rebuild removes it, and the client then reports a timeout rather than a
  // missing file. The write is refused in Rust; this is so the reason arrives
  // before the click, and covers copying the snippet by hand too (SPEC §15).
  const developmentBuild = status?.developmentBuild ?? false;

  useEffect(() => {
    if (!vault) return;
    listClientConfigs(vault).then(setClients).catch(() => setClients([]));
  }, [vault, installed]);

  if (!vault) {
    return (
      <Panel title="No vault selected">
        <p className="muted">Create or choose a vault on the Setup screen first.</p>
      </Panel>
    );
  }

  async function check() {
    setBusy(true);
    setError(null);
    try {
      setHealth(await healthCheck(vault));
    } finally {
      setBusy(false);
    }
  }

  async function install(clientId: string) {
    setBusy(true);
    setError(null);
    try {
      setInstalled(await installClientConfig(clientId, vault));
    } catch (failure) {
      setError(failure as DomainError);
    } finally {
      setBusy(false);
    }
  }

  return (
    <>
      <Panel
        title="Run the server"
        description="One server process per vault. The vault is fixed by this command and no tool call can change it."
      >
        <pre className="snippet">{mcpCommand(cliPath, vault)}</pre>
      </Panel>

      <Panel
        title="Client configuration"
        description="Paste this into your AI client's MCP configuration. It points at the copy of the command line tool bundled with this application."
      >
        <pre className="snippet">{clientConfigSnippet(cliPath, vault, serverKey)}</pre>
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
        description="Writes the entry above into a detected client's configuration. Existing servers and settings are kept, and the previous file is backed up first."
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
                      : client.installed
                        ? " — already configured"
                        : client.present
                          ? ""
                          : " — not installed"}
                  </span>
                  <span className="field__hint">{client.path}</span>
                </div>
                <Button onClick={() => install(client.id)} disabled={busy || developmentBuild}>
                  {client.installed ? "Update entry" : "Add entry"}
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
              ...(installed.backupPath
                ? ([["Backup", installed.backupPath]] as [string, string][])
                : []),
            ]}
          />
        </Notice>
      ) : null}

      <Panel
        title="Health check"
        description="Starts the server and completes a real MCP handshake against it — the same thing your client does."
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

/**
 * Diagnostics — what is actually running, and what recently went wrong
 * (SPEC §15).
 *
 * The point of this screen is to answer "which binary is this using?" without
 * guessing, and to keep failures readable instead of transient.
 */

import { inspectVault } from "../../api/cli";
import type { CliStatus, DomainError } from "../../api/types";
import { Button, Facts, Failure, Panel } from "../../components";
import { useState } from "react";

/** Bounded so a long session cannot grow this without limit. */
const RECENT_LIMIT = 10;

export interface RecordedFailure {
  at: string;
  context: string;
  error: DomainError;
}

/** Limits the CLI enforces, restated so a user can see them before hitting one. */
const LIMITS: [string, string][] = [
  ["Listing page", "50 default, 200 maximum"],
  ["Recursive depth", "4 default, 16 maximum"],
  ["Listing scan guard", "10,000 entries per call"],
  ["Documents per read", "10"],
  ["Lines per file", "200 default, 1,000 maximum"],
  ["Bytes per read", "64 KiB default, 256 KiB maximum"],
  ["Main memory", "32 KiB limit, advisory from 24 KiB"],
  ["Extended memory", "1 MiB"],
  ["Entry content", "1 MiB"],
];

export function Diagnostics({
  vault,
  status,
  failures,
  onRefresh,
}: {
  vault: string;
  status: CliStatus | null;
  failures: RecordedFailure[];
  onRefresh: () => void;
}) {
  const [vaultState, setVaultState] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  async function checkVault() {
    if (!vault) return;
    setBusy(true);
    try {
      const response = await inspectVault(vault);
      setVaultState(
        response.ok ? "initialized" : (response.error?.code ?? "unknown failure"),
      );
    } finally {
      setBusy(false);
    }
  }

  return (
    <>
      <Panel
        title="Command line tool"
        description="This application always runs its own bundled copy. A separately installed heimdall simply coexists."
        action={<Button onClick={onRefresh}>Refresh</Button>}
      >
        <Facts
          rows={[
            ["Path", status?.path ?? "resolving…"],
            ["Available", status?.available ? "yes" : "no"],
            ["CLI version", status?.cliVersion ?? "—"],
            ["Core version", status?.coreVersion ?? "—"],
            ["MCP protocol", status?.mcpProtocolVersion ?? "—"],
            ["Output schema", status?.outputSchemaVersion?.toString() ?? "—"],
          ]}
        />
        {status?.error ? <Failure error={status.error} /> : null}
      </Panel>

      <Panel
        title="Vault"
        action={
          <Button onClick={checkVault} disabled={busy || !vault}>
            {busy ? "Checking…" : "Check"}
          </Button>
        }
      >
        <Facts
          rows={[
            ["Path", vault || "none selected"],
            ["Status", vaultState ?? "not checked"],
          ]}
        />
      </Panel>

      <Panel
        title="Limits"
        description="Enforced by the command line tool, not by this application."
      >
        <Facts rows={LIMITS} />
      </Panel>

      <Panel title="Recent errors">
        {failures.length === 0 ? (
          <p className="muted">Nothing has failed in this session.</p>
        ) : (
          <div className="stack">
            {failures.slice(0, RECENT_LIMIT).map((failure, index) => (
              <div className="stack" key={`${failure.at}-${index}`}>
                <span className="field__hint">
                  {failure.at} — {failure.context}
                </span>
                <Failure error={failure.error} />
              </div>
            ))}
          </div>
        )}
      </Panel>
    </>
  );
}

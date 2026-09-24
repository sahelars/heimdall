/**
 * Setup — create a templated vault, or open an existing folder of notes as one.
 *
 * This screen sets a vault up; it never browses or edits its content. The
 * workspace does that (SPEC §15).
 */

import { useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";

import { createVault, inspectVault, vaultName } from "../../api/cli";
import type { CreateVaultData, DomainError } from "../../api/types";
import { Button, Facts, Failure, Field, Notice, Panel } from "../../components";

export function Setup({
  vault,
  onVaultChange,
}: {
  vault: string;
  onVaultChange: (vault: string) => void;
}) {
  const [name, setName] = useState("");
  const [root, setRoot] = useState("");
  const [busy, setBusy] = useState(false);
  const [created, setCreated] = useState<CreateVaultData | null>(null);
  const [error, setError] = useState<DomainError | null>(null);
  const [checked, setChecked] = useState<string | null>(null);

  async function pick(setter: (value: string) => void) {
    const chosen = await open({ directory: true, multiple: false });
    if (typeof chosen === "string") setter(chosen);
  }

  async function create() {
    setBusy(true);
    setError(null);
    setCreated(null);
    setChecked(null);
    try {
      const response = await createVault(name.trim(), root.trim());
      if (response.ok && response.data) {
        setCreated(response.data);
        onVaultChange(response.data.path);
      } else if (response.error) {
        setError(response.error);
      }
    } finally {
      setBusy(false);
    }
  }

  /**
   * Opening an existing folder is the same `create` call with the folder split
   * into its parent and name. A folder that already holds notes is registered
   * as a vault and nothing is written into it (SPEC §7).
   */
  async function openExisting() {
    const chosen = await open({ directory: true, multiple: false });
    if (typeof chosen !== "string") return;

    const trimmed = chosen.replace(/[/\\]+$/, "");
    const separator = trimmed.lastIndexOf("/") >= 0 ? "/" : "\\";
    const parent = trimmed.slice(0, trimmed.lastIndexOf(separator)) || separator;
    const folder = vaultName(trimmed);

    setBusy(true);
    setError(null);
    setCreated(null);
    setChecked(null);
    try {
      const response = await createVault(folder, parent);
      if (response.ok && response.data) {
        setCreated(response.data);
        onVaultChange(response.data.path);
      } else if (response.error) {
        setError(response.error);
      }
    } finally {
      setBusy(false);
    }
  }

  async function verify() {
    setBusy(true);
    setError(null);
    setChecked(null);
    try {
      const response = await inspectVault(vault);
      if (response.ok && response.data) {
        const count = response.data.listing?.entries.length ?? 0;
        const more = response.data.listing?.next_cursor ? "+" : "";
        setChecked(
          `Readable: ${count}${more} item${count === 1 && !more ? "" : "s"} at the vault root${
            response.data.locked ? ", locked" : ""
          }`,
        );
      } else if (response.error) {
        setError(response.error);
      }
    } finally {
      setBusy(false);
    }
  }

  return (
    <>
      <Panel
        title="Create a vault"
        description="Creates a new folder with a few example notes to start from."
      >
        {/*
          * The hint sits under the row rather than inside the first field.
          *
          * A row aligns its items along their bottoms, so a field carrying a
          * line of hint text is taller than the one beside it and rides up:
          * "Vault name" and its box sat a line above "Location" and its box.
          * Out here the hint costs the row no height, and the two labels, the
          * two inputs and the two buttons each share a line.
          */}
        <div className="row">
          <Field
            id="setup-name"
            label="Vault name"
            value={name}
            onChange={setName}
            placeholder="my-vault"
          />
          <Field
            id="setup-root"
            label="Location"
            value={root}
            onChange={setRoot}
            placeholder="/Users/you/Documents"
            grow
          />
          <Button onClick={() => pick(setRoot)}>Choose…</Button>
          <Button primary onClick={create} disabled={busy || !name.trim() || !root.trim()}>
            {busy ? "Working…" : "Create vault"}
          </Button>
        </div>
        <span className="field__hint">One folder name, not a path.</span>
      </Panel>

      <Panel
        title="Use an existing folder"
        description="Opens a folder of notes you already have as a vault. Nothing is written into it: no example notes, no hidden files."
      >
        <div className="row">
          <Button onClick={openExisting} disabled={busy}>
            Choose a folder…
          </Button>
        </div>
      </Panel>

      {created ? (
        <Notice
          title={created.mode === "scaffolded" ? "Vault created" : "Vault opened"}
        >
          <Facts
            rows={[
              ["Path", created.path],
              [
                "Written",
                created.created.length > 0
                  ? `${created.created.length} item(s)`
                  : "nothing — the folder is used as it is",
              ],
            ]}
          />
          {created.created.length > 0 ? (
            <ul className="list">
              {created.created.map((entry) => (
                <li key={entry}>{entry}</li>
              ))}
            </ul>
          ) : null}
        </Notice>
      ) : null}

      {vault ? (
        <Panel title="Active vault" action={<Button onClick={verify} disabled={busy}>Verify</Button>}>
          <Facts rows={[["Path", vault]]} />
          {checked ? <p className="muted">{checked}</p> : null}
        </Panel>
      ) : null}

      {error ? <Failure error={error} /> : null}
    </>
  );
}

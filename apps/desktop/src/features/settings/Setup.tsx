/**
 * Setup — create a templated vault, or open an existing folder of notes as one.
 *
 * This screen sets a vault up; it never browses or edits its content. The
 * workspace does that (SPEC §15).
 *
 * A vault made or opened here is shared with AI clients unless the switch
 * beside the button is turned off: someone who has just made a vault and then
 * asks ChatGPT to write in it means for that to work. The Active vault panel
 * switches it either way afterwards.
 */

import { useCallback, useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";

import {
  createVault,
  inspectVault,
  listVaults,
  protectedLocation,
  shareVault,
  unshareVault,
  vaultName,
} from "../../api/cli";
import type { CreateVaultData, DomainError, KnownVault } from "../../api/types";
import { Button, Facts, Failure, Field, Notice, Panel, Switch } from "../../components";

const SHARE_HINT = "ChatGPT and Claude can read and write it; locks still apply.";

/** What to say under a location macOS keeps AI clients out of. */
function protectedHint(place: string): string {
  return `macOS keeps AI clients out of ${place} unless Heimdall's command line tool has Full Disk Access. A folder in your home directory, such as ~/Heimdall, avoids this.`;
}

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
  // Whether a vault made or opened here is shared with AI clients.
  const [share, setShare] = useState(true);
  // What the last create or open shared it as; null when it was not shared.
  const [sharedAs, setSharedAs] = useState<string | null>(null);
  const [opened, setOpened] = useState<string | null>(null);
  // The active vault's entry in `heimdall vaults`, for its switch.
  const [active, setActive] = useState<KnownVault | null>(null);

  const refreshActive = useCallback(async () => {
    if (!vault) {
      setActive(null);
      return;
    }
    try {
      const response = await listVaults();
      const known = response.ok
        ? response.data?.vaults.find((entry) => entry.path === vault)
        : undefined;
      setActive(known ?? null);
    } catch {
      setActive(null);
    }
  }, [vault]);

  useEffect(() => {
    void refreshActive();
  }, [refreshActive]);

  async function pick(setter: (value: string) => void) {
    const chosen = await open({ directory: true, multiple: false });
    if (typeof chosen === "string") setter(chosen);
  }

  /**
   * Make or adopt a vault, open it, and share it if the switch says so.
   *
   * A share that fails is reported, but the vault is still made and opened:
   * the vault is the thing asked for, and it can be shared from the Active
   * vault panel once whatever stopped it is fixed.
   */
  async function setUp(folder: string, parent: string) {
    setBusy(true);
    setError(null);
    setCreated(null);
    setChecked(null);
    setSharedAs(null);
    try {
      const response = await createVault(folder, parent);
      if (!response.ok || !response.data) {
        if (response.error) setError(response.error);
        return;
      }
      setCreated(response.data);
      setOpened(response.data.path);
      if (share) {
        const shared = await shareVault(response.data.path);
        if (shared.ok && shared.data) setSharedAs(shared.data.name);
        else if (shared.error) setError(shared.error);
      }
      onVaultChange(response.data.path);
    } finally {
      setBusy(false);
    }
  }

  function create() {
    return setUp(name.trim(), root.trim());
  }

  async function toggleActive(on: boolean) {
    if (!vault) return;
    setBusy(true);
    setError(null);
    try {
      const response = on ? await shareVault(vault) : await unshareVault(vault);
      if (!response.ok && response.error) setError(response.error);
      await refreshActive();
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
    await setUp(folder, parent);
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
            placeholder="/Users/you/Heimdall"
            grow
          />
          <Button onClick={() => pick(setRoot)}>Choose…</Button>
          <Button primary onClick={create} disabled={busy || !name.trim() || !root.trim()}>
            {busy ? "Working…" : "Create vault"}
          </Button>
        </div>
        <span className="field__hint">One folder name, not a path.</span>
        {protectedLocation(root.trim()) ? (
          <span className="field__hint">{protectedHint(protectedLocation(root.trim())!)}</span>
        ) : null}
        <Switch
          id="setup-share"
          label="Available to AI clients"
          checked={share}
          onChange={setShare}
          hint={SHARE_HINT}
        />
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
        <Switch
          id="setup-share-existing"
          label="Available to AI clients"
          checked={share}
          onChange={setShare}
          hint={SHARE_HINT}
        />
      </Panel>

      {created ? (
        <Notice
          title={created.mode === "scaffolded" ? "Vault created" : "Vault opened"}
        >
          <Facts
            rows={[
              ["Path", created.path],
              ["AI clients", sharedAs ? `shared as “${sharedAs}”` : "not shared"],
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
          {opened && protectedLocation(opened) ? (
            <p className="muted">{protectedHint(protectedLocation(opened)!)}</p>
          ) : null}
        </Notice>
      ) : null}

      {vault ? (
        <Panel title="Active vault" action={<Button onClick={verify} disabled={busy}>Verify</Button>}>
          <Facts rows={[["Path", vault]]} />
          {checked ? <p className="muted">{checked}</p> : null}
          <Switch
            id="active-share"
            label={
              active?.shared && active.name
                ? `Available to AI clients as “${active.name}”`
                : "Available to AI clients"
            }
            checked={Boolean(active?.shared)}
            onChange={(on) => void toggleActive(on)}
            disabled={busy}
            hint={SHARE_HINT}
          />
          {protectedLocation(vault) ? (
            <p className="muted">{protectedHint(protectedLocation(vault)!)}</p>
          ) : null}
        </Panel>
      ) : null}

      {error ? <Failure error={error} /> : null}
    </>
  );
}

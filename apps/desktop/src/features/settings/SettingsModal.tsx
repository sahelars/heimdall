/**
 * Settings, as a modal over the workspace.
 *
 * It is reached from the application menu — Heimdall → Settings…, or ⌘, — rather
 * than from a tab in the window, because setting up a vault and configuring a
 * server are things done occasionally, and the window belongs to the notes.
 */

import { useState } from "react";

import type { CliStatus } from "../../api/types";
import { Modal } from "../../components/Modal";
import type { ThemePreference } from "../../state/theme";
import { Appearance } from "./Appearance";
import { Diagnostics, type RecordedFailure } from "./Diagnostics";
import { Server } from "./Server";
import { Setup } from "./Setup";

const SECTIONS = ["Vault", "Server", "Appearance", "Diagnostics"] as const;
type Section = (typeof SECTIONS)[number];

interface SettingsModalProps {
  open: boolean;
  vault: string;
  status: CliStatus | null;
  failures: RecordedFailure[];
  theme: ThemePreference;
  onThemeChange: (preference: ThemePreference) => void;
  onVaultChange: (vault: string) => void;
  onRefresh: () => void;
  onClose: () => void;
}

export function SettingsModal(props: SettingsModalProps) {
  const [section, setSection] = useState<Section>("Vault");

  return (
    <Modal title="Settings" open={props.open} onClose={props.onClose}>
      <nav className="modal__tabs" aria-label="Settings sections">
        {SECTIONS.map((name) => (
          <button
            key={name}
            type="button"
            className="tab"
            aria-current={section === name ? "page" : undefined}
            onClick={() => setSection(name)}
          >
            {name}
          </button>
        ))}
      </nav>

      <div className="modal__body">
        {section === "Vault" ? (
          <Setup vault={props.vault} onVaultChange={props.onVaultChange} />
        ) : null}
        {section === "Server" ? <Server vault={props.vault} status={props.status} /> : null}
        {section === "Appearance" ? (
          <Appearance preference={props.theme} onChange={props.onThemeChange} />
        ) : null}
        {section === "Diagnostics" ? (
          <Diagnostics
            vault={props.vault}
            status={props.status}
            failures={props.failures}
            onRefresh={props.onRefresh}
          />
        ) : null}
      </div>
    </Modal>
  );
}

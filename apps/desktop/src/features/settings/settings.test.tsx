/**
 * Screen behavior (SPEC §17, desktop tests).
 *
 * Tauri commands are stubbed at the IPC boundary, so these exercise the real
 * components against the real shape of a CLI response — including the failure
 * shapes, which must stay readable rather than becoming "something went wrong".
 */

import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.fn();
const openDialog = vi.fn();
const listenEvent = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({ invoke: (...args: unknown[]) => invoke(...args) }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: (...args: unknown[]) => openDialog(...args) }));
// New: the workspace subscribes to the application menu's Settings event.
vi.mock("@tauri-apps/api/event", () => ({ listen: (...args: unknown[]) => listenEvent(...args) }));

const { App } = await import("../../App");
const { Setup } = await import("./Setup");
const { Server } = await import("./Server");
const { Diagnostics } = await import("./Diagnostics");

const STATUS = {
  path: "/Applications/Heimdall.app/Contents/MacOS/heimdall",
  available: true,
  developmentBuild: false,
  cliVersion: "0.1.0",
  coreVersion: "0.1.0",
  mcpProtocolVersion: "2025-11-25",
  outputSchemaVersion: 1,
};

/** The same application run from a build tree rather than an installed copy. */
const DEV_STATUS = {
  ...STATUS,
  path: "/Users/n/heimdall/apps/desktop/src-tauri/target/debug/heimdall",
  developmentBuild: true,
};

/** Captured so a test can fire the menu event the way Rust does. */
let fireSettings: (() => void) | undefined;

beforeEach(() => {
  invoke.mockReset();
  openDialog.mockReset();
  listenEvent.mockReset();
  fireSettings = undefined;
  listenEvent.mockImplementation((_name: string, handler: () => void) => {
    fireSettings = handler;
    return Promise.resolve(() => {});
  });
  window.localStorage.clear();
  document.documentElement.removeAttribute("data-theme");
  document.documentElement.removeAttribute("style");
});

describe("Setup", () => {
  it("creates a templated vault and reports what was written", async () => {
    invoke.mockResolvedValue({
      ok: true,
      data: {
        path: "/Users/n/Documents/demo",
        mode: "scaffolded",
        created: ["AGENTS.md", "ideas/hello_world.md"],
      },
    });
    const onVaultChange = vi.fn();
    render(<Setup vault="" onVaultChange={onVaultChange} />);

    await userEvent.type(screen.getByLabelText("Vault name"), "demo");
    await userEvent.type(screen.getByLabelText("Location"), "/Users/n/Documents");
    await userEvent.click(screen.getByRole("button", { name: "Create vault" }));

    await screen.findByText("Vault created");
    expect(invoke).toHaveBeenCalledWith("invoke_cli", {
      command: "create",
      request: { name: "demo", root: "/Users/n/Documents" },
      stdin: undefined,
    });
    expect(onVaultChange).toHaveBeenCalledWith("/Users/n/Documents/demo");
    expect(screen.getByText("ideas/hello_world.md")).toBeInTheDocument();
  });

  it("registers an existing folder by its parent and name, writing nothing into it", async () => {
    openDialog.mockResolvedValue("/Users/n/Documents/Existing Vault");
    invoke.mockResolvedValue({
      ok: true,
      data: {
        path: "/Users/n/Documents/Existing Vault",
        mode: "registered",
        created: [],
      },
    });
    const onVaultChange = vi.fn();
    render(<Setup vault="" onVaultChange={onVaultChange} />);

    await userEvent.click(screen.getByRole("button", { name: "Choose a folder…" }));

    await screen.findByText("Vault opened");
    expect(invoke).toHaveBeenCalledWith("invoke_cli", {
      command: "create",
      request: { name: "Existing Vault", root: "/Users/n/Documents" },
      stdin: undefined,
    });
    expect(onVaultChange).toHaveBeenCalledWith("/Users/n/Documents/Existing Vault");
    expect(screen.getByText(/the folder is used as it is/)).toBeInTheDocument();
  });

  it("keeps a structured failure readable instead of crashing", async () => {
    invoke.mockResolvedValue({
      ok: false,
      error: {
        code: "ALREADY_EXISTS",
        message: '"demo" already exists in the root directory and is not a directory',
        details: { name: "demo" },
      },
    });
    render(<Setup vault="" onVaultChange={vi.fn()} />);

    await userEvent.type(screen.getByLabelText("Vault name"), "demo");
    await userEvent.type(screen.getByLabelText("Location"), "/tmp");
    await userEvent.click(screen.getByRole("button", { name: "Create vault" }));

    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("ALREADY_EXISTS");
    expect(alert).toHaveTextContent("already exists in the root directory");
    // The details a user would need to act are still there.
    expect(alert).toHaveTextContent("demo");
  });

  it("verifies the active vault by reading its root", async () => {
    invoke.mockResolvedValue({
      ok: true,
      data: {
        path: "",
        kind: "directory",
        locked: false,
        listing: {
          entries: [
            { path: "ideas", kind: "directory", modified_at: "", locked: false },
            { path: "README.md", kind: "document", modified_at: "", locked: false },
          ],
          next_cursor: null,
          scan_guard_hit: false,
        },
      },
    });
    render(<Setup vault="/Users/n/notes" onVaultChange={vi.fn()} />);

    await userEvent.click(screen.getByRole("button", { name: "Verify" }));

    await screen.findByText("Readable: 2 items at the vault root");
    expect(invoke).toHaveBeenCalledWith("invoke_cli", {
      command: "read",
      request: { vault: "/Users/n/notes" },
      stdin: undefined,
    });
  });

  it("surfaces a vault that cannot be read with the message the CLI gives", async () => {
    invoke.mockResolvedValue({
      ok: false,
      error: {
        code: "NOT_FOUND",
        message: 'the vault "/Users/n/moved" does not exist',
        details: { path: "/Users/n/moved" },
      },
    });
    render(<Setup vault="/Users/n/moved" onVaultChange={vi.fn()} />);

    await userEvent.click(screen.getByRole("button", { name: "Verify" }));

    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("NOT_FOUND");
    expect(alert).toHaveTextContent("does not exist");
  });
});

describe("Server", () => {
  const CLAUDE = {
    id: "claude-desktop",
    name: "Claude Desktop",
    path: "/Users/n/Library/Application Support/Claude/claude_desktop_config.json",
    present: true,
    installed: false,
    stale: false,
    legacy: [],
  };
  const CHATGPT = {
    id: "chatgpt",
    name: "ChatGPT",
    path: "/Users/n/.codex/config.toml",
    present: true,
    installed: false,
    stale: false,
    legacy: [],
  };

  /** Answer the Tauri commands the Server screen makes, with these fixtures. */
  function serve({
    clients = [] as unknown[],
    vaults = [] as unknown[],
    health = null as unknown,
    installed = null as unknown,
    onCli = (_command: string, _request: Record<string, unknown>) => undefined as unknown,
  } = {}) {
    invoke.mockImplementation((command: string, args?: Record<string, unknown>) => {
      if (command === "list_client_configs") return Promise.resolve(clients);
      if (command === "health_check") return Promise.resolve(health);
      if (command === "install_client_config") return Promise.resolve(installed);
      if (command === "invoke_cli") {
        const cli = args?.command as string;
        const request = (args?.request ?? {}) as Record<string, unknown>;
        const answer = onCli(cli, request);
        if (answer !== undefined) return Promise.resolve(answer);
        if (cli === "vaults") return Promise.resolve({ ok: true, data: { vaults } });
        return Promise.resolve({ ok: true, data: {} });
      }
      return Promise.resolve(null);
    });
  }

  it("shows one command and one entry, with no vault in either", async () => {
    serve();
    render(<Server vault="/Users/n/My Vault" status={STATUS} />);

    expect(screen.getByText(`${STATUS.path} mcp`)).toBeInTheDocument();
    const snippet = screen.getByText(/"mcpServers"/);
    const parsed = JSON.parse(snippet.textContent!);
    expect(parsed.mcpServers).toEqual({ heimdall: { command: STATUS.path, args: ["mcp"] } });
    expect(screen.getByText(/args = \["mcp"\]/)).toBeInTheDocument();
  });

  it("lists vaults with their shared names and shares one only when asked", async () => {
    const shareCalls: Record<string, unknown>[] = [];
    serve({
      vaults: [
        { path: "/Users/n/Work", folder: "Work", exists: true, shared: true, name: "Work" },
        { path: "/Users/n/Private", folder: "Private", exists: true, shared: false, name: null },
      ],
      onCli: (command, request) => {
        if (command === "share") shareCalls.push(request);
        return undefined;
      },
    });
    render(<Server vault="/Users/n/Work" status={STATUS} />);

    expect(await screen.findByText("Work — shared")).toBeInTheDocument();
    expect(screen.getByText("Private — not shared")).toBeInTheDocument();
    expect(shareCalls).toHaveLength(0);

    await userEvent.click(screen.getByRole("button", { name: "Share" }));
    await waitFor(() => expect(shareCalls).toEqual([{ vault: "/Users/n/Private" }]));
  });

  it("offers the open vault for sharing before anything has registered it", async () => {
    serve({ vaults: [] });
    render(<Server vault="/Users/n/Fresh" status={STATUS} />);
    expect(await screen.findByText("Fresh — not shared")).toBeInTheDocument();
  });

  it("warns that macOS may keep a client out of a vault in Documents", async () => {
    serve({
      vaults: [
        {
          path: "/Users/n/Documents/Heimdall-Vault",
          folder: "Heimdall-Vault",
          exists: true,
          shared: true,
          name: "Heimdall-Vault",
        },
      ],
    });
    render(<Server vault="/Users/n/Documents/Heimdall-Vault" status={STATUS} />);

    expect(await screen.findByText("macOS may keep AI clients out")).toBeInTheDocument();
    expect(screen.getByText(/is in Documents/)).toBeInTheDocument();
    expect(screen.getByText(/Privacy & Security/)).toBeInTheDocument();
  });

  it("reports a successful handshake with what the server said and why it cannot prove more", async () => {
    serve({
      health: {
        ok: true,
        serverName: "heimdall",
        serverVersion: "0.1.0",
        protocolVersion: "2025-11-25",
      },
    });
    render(<Server vault="/v" status={STATUS} />);

    expect(screen.getByText(/runs with Heimdall's own macOS permissions/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Run health check" }));

    await screen.findByText("Handshake succeeded");
    expect(screen.getByText("heimdall 0.1.0")).toBeInTheDocument();
    expect(screen.getByText("2025-11-25")).toBeInTheDocument();
    expect(invoke).toHaveBeenCalledWith("health_check");
  });

  it("keeps a failed handshake actionable", async () => {
    serve({
      health: {
        ok: false,
        error: { code: "IO_ERROR", message: "the server did not answer within 10 seconds" },
        stderr: "heimdall mcp: could not start",
      },
    });
    render(<Server vault="/missing" status={STATUS} />);

    await userEvent.click(screen.getByRole("button", { name: "Run health check" }));

    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("IO_ERROR");
    expect(alert).toHaveTextContent("did not answer");
    // Captured stderr is shown, because that is where the real reason is.
    expect(alert).toHaveTextContent("could not start");
  });

  it("refuses to put a development build's path into a client's configuration", async () => {
    // The path would break on the next rebuild, and the client's only symptom
    // would be a timeout — so the reason has to arrive before the click.
    serve({ clients: [CLAUDE] });
    render(<Server vault="/v" status={DEV_STATUS} />);

    expect(await screen.findByText("Not available in a development build")).toBeInTheDocument();
    expect(screen.getByText("Development build")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Add entry" })).toBeDisabled();
  });

  it("says when the entry already there names a command that has gone", async () => {
    serve({ clients: [{ ...CLAUDE, installed: true, stale: true }] });
    render(<Server vault="/v" status={STATUS} />);

    expect(await screen.findByText(/its command is gone/)).toBeInTheDocument();
  });

  it("names the per-vault entries an update replaces", async () => {
    serve({
      clients: [
        {
          ...CHATGPT,
          legacy: [
            { key: "heimdall", vault: "/Users/n/Documents/Vault-test" },
            { key: "heimdall-heimdall-vault", vault: "/Users/n/Documents/Heimdall-Vault" },
          ],
        },
      ],
    });
    render(<Server vault="/v" status={STATUS} />);

    expect(
      await screen.findByText(/Replaces 2 older per-vault entries: heimdall, heimdall-heimdall-vault/),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Update entry" })).toBeEnabled();
  });

  it("will not overwrite a heimdall entry that runs something else", async () => {
    serve({ clients: [{ ...CLAUDE, conflict: "\"heimdall\" in this file runs something other than Heimdall" }] });
    render(<Server vault="/v" status={STATUS} />);

    expect(await screen.findByText(/runs something other than Heimdall/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Add entry" })).toBeDisabled();
  });

  it("writes a client entry only when asked, and says what it did", async () => {
    serve({
      clients: [CLAUDE],
      installed: {
        path: CLAUDE.path,
        serverKey: "heimdall",
        replaced: true,
        removed: ["heimdall-work-notes"],
        backupPath: "/Users/n/Library/Application Support/Claude/claude_desktop_config.heimdall-backup.json",
      },
    });
    render(<Server vault="/v" status={STATUS} />);

    await screen.findByText(CLAUDE.path);
    // Nothing is written until the button is pressed.
    expect(invoke).not.toHaveBeenCalledWith("install_client_config", expect.anything());

    await userEvent.click(screen.getByRole("button", { name: "Add entry" }));

    await screen.findByText("Configuration written");
    expect(invoke).toHaveBeenCalledWith("install_client_config", { clientId: "claude-desktop" });
    expect(screen.getByText("heimdall-work-notes")).toBeInTheDocument();
    expect(screen.getByText(/heimdall-backup\.json/)).toBeInTheDocument();
  });

  it("offers ChatGPT alongside Claude Desktop, and says which ChatGPT modes can use it", async () => {
    serve({ clients: [CLAUDE, { ...CHATGPT, present: false }] });
    render(<Server vault="/v" status={STATUS} />);

    expect(await screen.findByText("ChatGPT — not installed")).toBeInTheDocument();
    expect(screen.getByText("/Users/n/.codex/config.toml")).toBeInTheDocument();
    expect(screen.getByText(/Work or Codex mode only/)).toBeInTheDocument();
    expect(screen.getByText(/Restart ChatGPT after adding/)).toBeInTheDocument();
    expect(screen.getAllByRole("button", { name: "Add entry" })).toHaveLength(2);
  });
});

describe("Diagnostics", () => {
  it("names the binary in use and the versions it reports", () => {
    render(
      <Diagnostics vault="/v" status={STATUS} failures={[]} onRefresh={vi.fn()} />,
    );

    expect(screen.getByText(STATUS.path)).toBeInTheDocument();
    expect(screen.getByText("2025-11-25")).toBeInTheDocument();
    expect(screen.getByText("Nothing has failed in this session.")).toBeInTheDocument();
  });

  it("reports a missing sidecar rather than pretending it is fine", () => {
    render(
      <Diagnostics
        vault="/v"
        status={{
          path: "/Applications/Heimdall.app/Contents/MacOS/heimdall",
          available: false,
          developmentBuild: false,
          error: {
            code: "IO_ERROR",
            message: "the bundled heimdall command line tool is missing from this application",
          },
        }}
        failures={[]}
        onRefresh={vi.fn()}
      />,
    );

    expect(screen.getByText("no")).toBeInTheDocument();
    expect(screen.getByRole("alert")).toHaveTextContent("IO_ERROR");
  });

  it("keeps recent failures where a user can still read them", () => {
    render(
      <Diagnostics
        vault="/v"
        status={STATUS}
        failures={[
          {
            at: "2026-08-17T10:00:00Z",
            context: "write ideas/note.md",
            error: {
              code: "REVISION_CONFLICT",
              message: "the note changed since it was read",
              details: { current_revision: "blake3:abc" },
            },
          },
        ]}
        onRefresh={vi.fn()}
      />,
    );

    const alert = screen.getByRole("alert");
    expect(alert).toHaveTextContent("REVISION_CONFLICT");
    expect(alert).toHaveTextContent("blake3:abc");
    expect(screen.getByText(/write ideas\/note\.md/)).toBeInTheDocument();
  });
});

describe("Settings", () => {
  it("is closed until the application menu asks for it, and closes again from the X", async () => {
    invoke.mockResolvedValue(STATUS);
    render(<App />);

    await waitFor(() =>
      expect(listenEvent).toHaveBeenCalledWith("menu:settings", expect.any(Function)),
    );
    // Settings is not a screen you can be stuck on.
    expect(screen.queryByRole("dialog")).toBeNull();

    act(() => fireSettings?.());
    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByRole("button", { name: "Create vault" })).toBeInTheDocument();

    await userEvent.click(within(dialog).getByRole("button", { name: "Close settings" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });

  it("moves between its sections without leaving the workspace", async () => {
    invoke.mockResolvedValue(STATUS);
    render(<App />);
    await waitFor(() => expect(listenEvent).toHaveBeenCalled());
    act(() => fireSettings?.());

    const dialog = await screen.findByRole("dialog");
    await userEvent.click(within(dialog).getByRole("button", { name: "Appearance" }));
    expect(within(dialog).getByRole("radiogroup", { name: "Theme" })).toBeInTheDocument();

    await userEvent.click(within(dialog).getByRole("button", { name: "Diagnostics" }));
    expect(within(dialog).getByText("Command line tool")).toBeInTheDocument();
  });

  it("opens without a ring drawn around its close button", async () => {
    // `showModal` focuses the first control it finds, which is the X — so the
    // sheet opened looking as though something in it had been selected.
    invoke.mockResolvedValue(STATUS);
    render(<App />);
    await waitFor(() => expect(listenEvent).toHaveBeenCalled());
    act(() => fireSettings?.());

    const dialog = await screen.findByRole("dialog");
    expect(document.activeElement).toBe(dialog);
    expect(document.activeElement).not.toBe(
      within(dialog).getByRole("button", { name: "Close settings" }),
    );
  });

  it("lines the vault fields up by keeping the hint out of their row", async () => {
    // A row aligns along the bottom, so a field carrying a hint is taller than
    // the one beside it and rides a line up. There is no layout in jsdom to
    // measure, but the cause is structural and can be checked.
    invoke.mockResolvedValue(STATUS);
    render(<App />);
    await waitFor(() => expect(listenEvent).toHaveBeenCalled());
    act(() => fireSettings?.());

    const dialog = await screen.findByRole("dialog");
    const hint = within(dialog).getByText("One folder name, not a path.");
    expect(hint.closest(".row")).toBeNull();
    // And it is still next to the field it describes, not orphaned elsewhere.
    const row = within(dialog).getByLabelText("Vault name").closest(".row");
    expect(row?.nextElementSibling).toBe(hint);
  });

  it("applies a theme override to the document and remembers it", async () => {
    invoke.mockResolvedValue(STATUS);
    render(<App />);
    await waitFor(() => expect(listenEvent).toHaveBeenCalled());
    act(() => fireSettings?.());

    const dialog = await screen.findByRole("dialog");
    await userEvent.click(within(dialog).getByRole("button", { name: "Appearance" }));
    await userEvent.click(within(dialog).getByRole("radio", { name: "Dark" }));

    expect(document.documentElement.getAttribute("data-theme")).toBe("dark");
    expect(window.localStorage.getItem("heimdall.theme")).toBe('"dark"');
  });

  async function appearance() {
    invoke.mockResolvedValue(STATUS);
    render(<App />);
    await waitFor(() => expect(listenEvent).toHaveBeenCalled());
    act(() => fireSettings?.());

    const dialog = await screen.findByRole("dialog");
    await userEvent.click(within(dialog).getByRole("button", { name: "Appearance" }));
    return dialog;
  }

  it("writes each theme's accent onto the root as its own property", async () => {
    // The stylesheet is not attached here, so what is asserted is the overrides
    // themselves: one inline custom property per theme. Which theme spends
    // which is the stylesheet's decision, not this component's — which is why
    // there is no theme to set up first.
    const dialog = await appearance();
    await userEvent.type(within(dialog).getByLabelText("Dark hex"), "#00ff00");

    expect(document.documentElement.style.getPropertyValue("--accent-dark")).toBe("#00ff00");
    expect(window.localStorage.getItem("heimdall.accent")).toBe(
      JSON.stringify({ light: null, dark: "#00ff00" }),
    );

    await userEvent.click(within(dialog).getAllByRole("button", { name: "Default" })[1]!);

    // Removed rather than written back: the default belongs to the stylesheet.
    expect(document.documentElement.style.getPropertyValue("--accent-dark")).toBe("");
  });

  it("keeps the two accents apart, so one theme's choice is not the other's", async () => {
    // The bug that made two accents necessary: a colour picked for dark mode
    // used to apply to light mode as well, where #ffffff is invisible.
    const dialog = await appearance();
    await userEvent.type(within(dialog).getByLabelText("Dark hex"), "#ffffff");

    expect(document.documentElement.style.getPropertyValue("--accent-dark")).toBe("#ffffff");
    expect(document.documentElement.style.getPropertyValue("--accent-light")).toBe("");

    await userEvent.type(within(dialog).getByLabelText("Light hex"), "#000000");

    expect(document.documentElement.style.getPropertyValue("--accent-light")).toBe("#000000");
    expect(document.documentElement.style.getPropertyValue("--accent-dark")).toBe("#ffffff");
  });

  /** Declare both bases on the document the way the production build ships them. */
  function well(dialog: HTMLElement, theme: "light" | "dark"): HTMLInputElement {
    return dialog.querySelector(`#accent-colour-${theme}`) as HTMLInputElement;
  }

  function shippedBases() {
    document.documentElement.style.setProperty("--accent-light-base", "#000");
    document.documentElement.style.setProperty("--accent-dark-base", "#fff");
  }

  it("fills both wells with their theme's default, as the built stylesheet spells it", async () => {
    // They came up empty in the shipped app: the minifier writes #000000 as
    // #000, and only six-digit hex was accepted.
    shippedBases();
    const dialog = await appearance();

    expect(within(dialog).getByLabelText("Light hex")).toHaveValue("#000000");
    expect(within(dialog).getByLabelText("Dark hex")).toHaveValue("#FFFFFF");
    expect(well(dialog, "light")).toHaveValue("#000000");
    expect(well(dialog, "dark")).toHaveValue("#ffffff");
  });

  it("shows the default again as soon as a choice is cleared", async () => {
    // The well used to read the document, which still held the cleared colour
    // until the effect that removes it ran.
    shippedBases();
    const dialog = await appearance();
    await userEvent.clear(within(dialog).getByLabelText("Dark hex"));
    await userEvent.type(within(dialog).getByLabelText("Dark hex"), "#00ff00");
    expect(well(dialog, "dark")).toHaveValue("#00ff00");

    await userEvent.click(within(dialog).getAllByRole("button", { name: "Default" })[1]!);

    expect(well(dialog, "dark")).toHaveValue("#ffffff");
    expect(within(dialog).getByLabelText("Dark hex")).toHaveValue("#FFFFFF");
    expect(within(dialog).getAllByRole("button", { name: "Default" })[1]).toBeDisabled();
  });

  it("draws each well on its own theme's ground", async () => {
    const dialog = await appearance();
    expect(well(dialog, "light")).toHaveClass("swatch", "swatch--light");
    expect(well(dialog, "dark")).toHaveClass("swatch", "swatch--dark");
  });

  it("does not offer to reset an accent that has not been set", async () => {
    const dialog = await appearance();

    for (const button of within(dialog).getAllByRole("button", { name: "Default" })) {
      expect(button).toBeDisabled();
    }
  });
});

describe("the workspace", () => {
  it("asks for a vault before it can show anything", async () => {
    invoke.mockResolvedValue(STATUS);
    render(<App />);

    expect(await screen.findByText(/No vault yet/)).toBeInTheDocument();
  });

  it("survives a bridge that rejects outright", async () => {
    invoke.mockRejectedValue(new Error("no bridge"));
    render(<App />);

    // The window still renders and still says what to do next.
    expect(await screen.findByText(/No vault yet/)).toBeInTheDocument();
  });
});

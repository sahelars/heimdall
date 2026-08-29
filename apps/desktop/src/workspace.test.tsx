/**
 * The workspace, driven by output the real CLI actually produced (SPEC §17).
 *
 * The fixtures in `src/test/fixtures/` were captured from `heimdall link-graph`,
 * `list-documents`, and `read-documents` run against a real vault, so this
 * exercises the whole render path against the real shape of the contract rather
 * than against a hand-written idea of it.
 */

import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import graphFixture from "./test/fixtures/link-graph.json";
import listingFixture from "./test/fixtures/list-documents.json";
import readFixture from "./test/fixtures/read-documents.json";
import type { RelinkData } from "./api/types";

const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...args: unknown[]) => invoke(...args) }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: () => Promise.resolve(() => {}) }));
// Mermaid measures text, which jsdom cannot do; the fence is what matters here.
vi.mock("./features/note/Mermaid", () => ({
  Mermaid: ({ code }: { code: string }) => <div data-testid="mermaid">{code}</div>,
}));

const { App } = await import("./App");

/** Comfortably more than two autosave debounces. */
const AUTOSAVE_WINDOWS = 3600;

/**
 * The note's title in preview, which is the heading it opens with.
 *
 * Static text here and a field in source: renaming the note and retitling it
 * are the same act, but preview is for reading.
 */
function titleOf(within_: HTMLElement) {
  return within_.querySelector(".preview__title")?.textContent ?? "";
}

const STATUS = {
  path: "/Applications/Heimdall.app/Contents/MacOS/heimdall",
  available: true,
  cliVersion: "0.1.0",
  coreVersion: "0.1.0",
  mcpProtocolVersion: "2025-11-25",
  outputSchemaVersion: 1,
};

/** Answer each bridge call with what the CLI really returned, or with a note
 * of the caller's own when what is under test is the note itself. */
/** What the `relink` that follows every move should answer, if a test cares. */
interface Extras {
  relink?: RelinkData;
  relinkFails?: { code: string; message: string };
}

const EMPTY_TRUNCATION = {
  file_cap_hit: false,
  node_cap_hit: false,
  total_bytes_cap_hit: false,
  nodes_omitted: 0,
  files_unscanned: 0,
  scanned_bytes: 0,
};

/** Every CLI subcommand the app has asked for, in order. */
function commandsCalled(): string[] {
  return invoke.mock.calls
    .filter((call) => call[0] === "invoke_cli")
    .map((call) => (call[1] as { command: string }).command);
}

function bridge(content?: string, extra: Extras = {}) {
  invoke.mockImplementation((command: string, args: Record<string, unknown>) => {
    if (command === "cli_status") return Promise.resolve(STATUS);
    if (command !== "invoke_cli") return Promise.resolve({ ok: true, data: {} });

    const request = args as { command: string; request: Record<string, unknown> };
    switch (request.command) {
      case "link-graph":
        return Promise.resolve({ ok: true, data: graphFixture });
      case "list-documents":
        return Promise.resolve({ ok: true, data: listingFixture });
      case "relink":
        if (extra.relinkFails) {
          return Promise.resolve({ ok: false, error: { ...extra.relinkFails, details: {} } });
        }
        return Promise.resolve({
          ok: true,
          data:
            extra.relink ??
            ({
              from: String(request.request.from ?? ""),
              to: String(request.request.to ?? ""),
              dry_run: false,
              updated: [],
              skipped: [],
              truncated: EMPTY_TRUNCATION,
            } satisfies RelinkData),
        });
      case "read-documents":
        return Promise.resolve({
          ok: true,
          data:
            content === undefined
              ? readFixture
              : {
                  ...readFixture,
                  documents: [
                    {
                      ...readFixture.documents[0],
                      content,
                      // The reader checks what it assembled against this, so a
                      // note supplied here has to report its own length.
                      size_bytes: new TextEncoder().encode(content).length,
                    },
                  ],
                },
        });
      default:
        return Promise.resolve({ ok: true, data: {} });
    }
  });
}

beforeEach(() => {
  invoke.mockReset();
  window.localStorage.clear();
  window.localStorage.setItem("heimdall.vault", JSON.stringify("/vault"));
  document.documentElement.removeAttribute("data-theme");
});

describe("the three panes", () => {
  it("shows the ordinary tree and the protected tree side by side", async () => {
    bridge();
    render(<App />);

    const files = await screen.findByRole("region", { name: "Files" });
    // `list-documents` never returns anything under aios/, so the protected
    // tree can only be here because the link index supplied it.
    expect(await within(files).findByText("aios")).toBeInTheDocument();
    expect(within(files).getByText("memories")).toBeInTheDocument();
    expect(within(files).getByText("extended")).toBeInTheDocument();
    expect(within(files).getByText("memory_1")).toBeInTheDocument();
    expect(within(files).getByText("notifications")).toBeInTheDocument();

    expect(within(files).getByText("projects")).toBeInTheDocument();
    expect(within(files).getByText("how_lens_works")).toBeInTheDocument();
  });

  it("renders all three panes at once", async () => {
    bridge();
    render(<App />);

    expect(await screen.findByRole("region", { name: "Files" })).toBeInTheDocument();
    expect(screen.getByRole("region", { name: "Note" })).toBeInTheDocument();
    expect(screen.getByRole("region", { name: "Graph" })).toBeInTheDocument();
    expect(screen.getByRole("img", { name: "Link graph" })).toBeInTheDocument();
  });

  it("opens a note into the preview with its properties and diagram", async () => {
    bridge();
    render(<App />);

    const files = await screen.findByRole("region", { name: "Files" });
    await userEvent.click(within(files).getByText("how_lens_works"));

    const note = screen.getByRole("region", { name: "Note" });
    await waitFor(() => expect(titleOf(note)).toBe("how_lens_works"));

    // Frontmatter renders as the Properties table, with its wikilinks clickable.
    const properties = within(note).getByRole("region", { name: "Properties" });
    expect(within(properties).getByText("links")).toBeInTheDocument();
    expect(within(properties).getByRole("button", { name: "profile" })).toBeInTheDocument();
    expect(within(properties).getByRole("button", { name: "articles" })).toBeInTheDocument();

    // The body renders, and the mermaid fence reaches the diagram component
    // rather than being shown as code.
    expect(within(note).getByRole("heading", { name: "Create Account Pipeline" })).toBeInTheDocument();
    expect(within(note).getByTestId("mermaid")).toHaveTextContent("graph TD");
  });

  it("shows the note's linked mentions, computed from the index", async () => {
    bridge();
    render(<App />);

    const files = await screen.findByRole("region", { name: "Files" });
    await userEvent.click(within(files).getByText("how_lens_works"));

    const mentions = await screen.findByRole("region", { name: "Linked mentions" });
    // `profile.md` and `articles.md` both link back to it in the real vault.
    expect(within(mentions).getByRole("button", { name: "profile" })).toBeInTheDocument();
    expect(within(mentions).getByRole("button", { name: "articles" })).toBeInTheDocument();
    // The name leads, and its path sits beside it — nothing else.
    expect(within(mentions).getByText("projects/lens/profile.md")).toBeInTheDocument();
  });

  it("does not repeat the note's own outgoing links under it", async () => {
    // They are already in the text a few lines above; a second copy is one more
    // list to read past rather than something new.
    bridge();
    render(<App />);

    const files = await screen.findByRole("region", { name: "Files" });
    await userEvent.click(within(files).getByText("how_lens_works"));

    await screen.findByRole("region", { name: "Linked mentions" });
    expect(screen.queryByRole("region", { name: "Outgoing links" })).toBeNull();
  });

  it("switches between the rendered note and its source", async () => {
    bridge();
    render(<App />);

    const files = await screen.findByRole("region", { name: "Files" });
    await userEvent.click(within(files).getByText("how_lens_works"));

    const note = screen.getByRole("region", { name: "Note" });
    await waitFor(() => expect(within(note).getByRole("region", { name: "Properties" })).toBeInTheDocument());

    await userEvent.click(within(note).getByRole("button", { name: "Edit" }));
    expect(within(note).getByTestId("source-editor")).toBeInTheDocument();

    await userEvent.click(within(note).getByRole("button", { name: "Read" }));
    expect(within(note).queryByTestId("source-editor")).toBeNull();
  });

  it("creates a note through its own dialog rather than a browser prompt", async () => {
    // wry implements neither the alert nor the text-input panel, so
    // `window.prompt` returns null without showing anything in the packaged
    // app — "New note" would quietly do nothing.
    bridge();
    render(<App />);

    const files = await screen.findByRole("region", { name: "Files" });
    await userEvent.click(within(files).getByRole("button", { name: "New note" }));

    const dialog = await screen.findByRole("dialog", { name: "Create note" });
    const input = within(dialog).getByLabelText("Note name");
    await userEvent.clear(input);
    await userEvent.type(input, "a_new_note");
    await userEvent.click(within(dialog).getByRole("button", { name: "Create note" }));

    await waitFor(() => {
      const write = invoke.mock.calls.find(
        (call) => (call[1] as { command?: string })?.command === "write-document",
      );
      expect(write?.[1]).toMatchObject({
        request: { path: "a_new_note.md", create: true },
        stdin: "# a_new_note\n\n",
      });
    });
  });

  it("saves an edit before opening another note, rather than dropping it", async () => {
    // Autosave is debounced, and switching notes cancels its pending timer. Any
    // edit made in the last second and a half before a click would otherwise be
    // gone with nothing said about it.
    bridge();
    render(<App />);

    const files = await screen.findByRole("region", { name: "Files" });
    await userEvent.click(within(files).getByText("how_lens_works"));

    const note = screen.getByRole("region", { name: "Note" });
    const properties = await within(note).findByRole("region", { name: "Properties" });

    // A real edit, through a path jsdom can drive: remove one frontmatter link.
    await userEvent.click(within(properties).getByRole("button", { name: "Remove articles" }));
    await waitFor(() =>
      expect(within(properties).queryByRole("button", { name: "articles" })).toBeNull(),
    );

    // Now navigate away well inside the autosave debounce.
    await userEvent.click(within(files).getByText("profile"));

    await waitFor(() => {
      const write = invoke.mock.calls.find(
        (call) => (call[1] as { command?: string })?.command === "write-document",
      );
      expect(write, "the edit was discarded instead of saved").toBeDefined();
      const request = write![1] as { request: { path: string }; stdin: string };
      expect(request.request.path).toBe("projects/lens/how_lens_works.md");
      expect(request.stdin).not.toContain("articles");
      expect(request.stdin).toContain("profile");
    });
  });

  it("finds a note by part of its name", async () => {
    bridge();
    render(<App />);
    await screen.findByRole("region", { name: "Files" });

    await userEvent.keyboard("{Meta>}o{/Meta}");
    const switcher = await screen.findByRole("dialog", { name: "Open note" });
    await userEvent.type(within(switcher).getByRole("textbox"), "hlw");

    const options = within(switcher).getAllByRole("option");
    expect(options[0]).toHaveTextContent("how_lens_works");
  });
});

describe("saving", () => {
  it("does not fire two writes at once when ⌘S lands twice in a tick", async () => {
    // CodeMirror binds Mod-s in its own keymap and the window handler binds it
    // too, so one keypress reaches both. Two writes carrying the same revision
    // means the second comes back as a conflict the user never caused.
    bridge();
    render(<App />);

    const files = await screen.findByRole("region", { name: "Files" });
    await userEvent.click(within(files).getByText("how_lens_works"));

    const note = screen.getByRole("region", { name: "Note" });
    const properties = await within(note).findByRole("region", { name: "Properties" });
    await userEvent.click(within(properties).getByRole("button", { name: "Remove articles" }));

    invoke.mockClear();
    // Both handlers, in the same tick.
    await userEvent.keyboard("{Meta>}s{/Meta}");
    await userEvent.keyboard("{Meta>}s{/Meta}");

    await waitFor(() => {
      const writes = invoke.mock.calls.filter(
        (call) => (call[1] as { command?: string })?.command === "write-document",
      );
      expect(writes.length).toBeLessThanOrEqual(1);
    });
    expect(within(note).queryByText(/Revision conflict/)).toBeNull();
  });
});

describe("keyboard shortcuts", () => {
  it("leaves the keyboard to whichever dialog is open", async () => {
    bridge();
    render(<App />);
    await screen.findByRole("region", { name: "Files" });

    await userEvent.keyboard("{Meta>}o{/Meta}");
    expect(await screen.findByRole("dialog", { name: "Open note" })).toBeInTheDocument();

    // ⌘, over the switcher must not stack Settings on top of it.
    await userEvent.keyboard("{Meta>},{/Meta}");
    expect(screen.queryByRole("dialog", { name: "Settings" })).toBeNull();

    // And the same key that opened it puts it away.
    await userEvent.keyboard("{Meta>}o{/Meta}");
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Open note" })).toBeNull());
  });
});

describe("opening notes in quick succession", () => {
  it("shows the note that was clicked last, not the read that finished last", async () => {
    // A large note takes several round trips. Without a guard, clicking a slow
    // note and then a fast one leaves the slow read landing last and replacing
    // what the user is actually looking at.
    let releaseSlow: (() => void) | undefined;
    invoke.mockImplementation((command: string, args: Record<string, unknown>) => {
      if (command === "cli_status") return Promise.resolve(STATUS);
      const request = args as { command: string; request: Record<string, string[]> };
      if (request.command === "link-graph") return Promise.resolve({ ok: true, data: graphFixture });
      if (request.command === "list-documents")
        return Promise.resolve({ ok: true, data: listingFixture });

      if (request.command === "read-documents") {
        const path = request.request.doc![0]!;
        // A note opens with its own name as its heading, which is what the
        // preview shows as the title.
        const body = `# ${path.split("/").pop()!.replace(/\.md$/, "")}\n`;
        const answer = {
          ok: true,
          data: {
            documents: [
              {
                path,
                returned: true,
                content: body,
                start_line: 1,
                end_line: 1,
                next_line: null,
                complete: true,
                size_bytes: new TextEncoder().encode(body).length,
                revision: "blake3:aa",
              },
            ],
            total_bytes: body.length,
            truncated: false,
          },
        };
        if (path.includes("how_lens_works")) {
          return new Promise((resolve) => {
            releaseSlow = () => resolve(answer);
          });
        }
        return Promise.resolve(answer);
      }
      return Promise.resolve({ ok: true, data: {} });
    });

    render(<App />);
    const files = await screen.findByRole("region", { name: "Files" });

    await userEvent.click(within(files).getByText("how_lens_works"));
    await userEvent.click(within(files).getByText("test_note"));

    const note = screen.getByRole("region", { name: "Note" });
    await waitFor(() => expect(titleOf(note)).toBe("test_note"));

    // The slow read lands only now.
    act(() => releaseSlow?.());

    await waitFor(() => expect(titleOf(note)).toBe("test_note"));
    expect(titleOf(note)).not.toBe("how_lens_works");
  });
});

describe("history", () => {
  it("steps back once per click, even when the clicks are fast", async () => {
    // Both handlers used to read the history from their own render, so two
    // clicks in a row computed the same single step from the same start.
    bridge();
    render(<App />);

    const files = await screen.findByRole("region", { name: "Files" });
    const note = screen.getByRole("region", { name: "Note" });

    await userEvent.click(within(files).getByText("hello_world"));
    await waitFor(() => expect(titleOf(note)).toBe("hello_world"));
    await userEvent.click(within(files).getByText("test_note"));
    await waitFor(() => expect(titleOf(note)).toBe("test_note"));
    await userEvent.click(within(files).getByText("game_idea"));
    await waitFor(() => expect(titleOf(note)).toBe("game_idea"));

    const back = within(note).getByRole("button", { name: "Back" });
    await userEvent.click(back);
    await userEvent.click(back);

    await waitFor(() => expect(titleOf(note)).toBe("hello_world"));

    // And forward returns the same way.
    await userEvent.click(within(note).getByRole("button", { name: "Forward" }));
    await waitFor(() => expect(titleOf(note)).toBe("test_note"));
  });
});

describe("renaming and deleting", () => {
  async function rightClick(name: string) {
    const files = await screen.findByRole("region", { name: "Files" });
    fireEvent.contextMenu(within(files).getByText(name));
    return screen.findByRole("menu");
  }

  /** Right-click a tree row, choose Rename…, and commit a new name. */
  async function renameFromTree(row: string, name: string) {
    const menu = await rightClick(row);
    await userEvent.click(within(menu).getByRole("menuitem", { name: "Rename…" }));

    const dialog = await screen.findByRole("dialog", { name: "Rename" });
    const field = within(dialog).getByLabelText("Note name");
    await userEvent.clear(field);
    await userEvent.type(field, name);
    await userEvent.click(within(dialog).getByRole("button", { name: "Rename" }));
  }

  it("offers rename and delete on an ordinary note", async () => {
    bridge();
    render(<App />);

    const menu = await rightClick("how_lens_works");
    expect(within(menu).getByRole("menuitem", { name: "Rename…" })).toBeInTheDocument();
    expect(within(menu).getByRole("menuitem", { name: "Delete…" })).toBeInTheDocument();
  });

  it("offers nothing for the protected tree, which Heimdall owns", async () => {
    // `move-path` and `delete-path` both refuse `aios/`, so offering the option
    // would only produce an error the user could do nothing about.
    bridge();
    render(<App />);

    const files = await screen.findByRole("region", { name: "Files" });
    fireEvent.contextMenu(within(files).getByText("memory_1"));
    expect(screen.queryByRole("menu")).toBeNull();
  });

  it("renames through move-path, keeping the extension", async () => {
    bridge();
    render(<App />);

    const menu = await rightClick("test_note");
    await userEvent.click(within(menu).getByRole("menuitem", { name: "Rename…" }));

    const dialog = await screen.findByRole("dialog", { name: "Rename" });
    const field = within(dialog).getByLabelText("Note name");
    expect(field).toHaveValue("test_note");

    await userEvent.clear(field);
    await userEvent.type(field, "renamed");
    await userEvent.click(within(dialog).getByRole("button", { name: "Rename" }));

    await waitFor(() => {
      const move = invoke.mock.calls.find(
        (call) => (call[1] as { command?: string })?.command === "move-path",
      );
      expect(move?.[1]).toMatchObject({
        request: { from: "ideas/test_note.md", to: "ideas/renamed.md" },
      });
    });
  });

  it("asks before it rewrites anything, and writes nothing until it is answered", async () => {
    // The whole point of the dialog: notes the user never opened are about to
    // change, so the rewrite has to be something they asked for by name.
    bridge();
    render(<App />);
    await renameFromTree("how_lens_works", "how_lens_work");

    const dialog = await screen.findByRole("dialog", { name: "Rename" });
    expect(within(dialog).getByText(/2 notes link here/)).toBeInTheDocument();
    // Named, not counted: the user is agreeing to these files specifically.
    expect(within(dialog).getByText("projects/lens/articles.md")).toBeInTheDocument();
    expect(within(dialog).getByText("projects/lens/profile.md")).toBeInTheDocument();

    // Nothing has happened yet — not even the rename.
    expect(commandsCalled()).not.toContain("move-path");
    expect(commandsCalled()).not.toContain("relink");
  });

  it("asks about links without a ring drawn around Cancel", async () => {
    // `showModal` focuses the first control it finds, which in this dialog is
    // Cancel — so the question arrived looking as though it had been answered.
    bridge();
    render(<App />);
    await renameFromTree("how_lens_works", "how_lens_work");

    const dialog = await screen.findByRole("dialog", { name: "Rename" });
    expect(document.activeElement).toBe(dialog);
    expect(document.activeElement).not.toBe(
      within(dialog).getByRole("button", { name: "Cancel" }),
    );
  });

  it("renames and carries the links when told to update them", async () => {
    bridge();
    render(<App />);
    await renameFromTree("how_lens_works", "how_lens_work");

    const dialog = await screen.findByRole("dialog", { name: "Rename" });
    await userEvent.click(within(dialog).getByRole("button", { name: "Update links" }));

    await waitFor(() => expect(commandsCalled()).toContain("relink"));
    // Order matters: rewriting first would retarget links at a path that does
    // not exist yet.
    const order = commandsCalled();
    expect(order.indexOf("move-path")).toBeLessThan(order.indexOf("relink"));

    const call = invoke.mock.calls.find(
      (entry) => (entry[1] as { command?: string })?.command === "relink",
    );
    expect(call?.[1]).toMatchObject({
      request: {
        from: "projects/lens/how_lens_works.md",
        to: "projects/lens/how_lens_work.md",
      },
    });
  });

  it("renames without touching other notes when told to rename only", async () => {
    bridge();
    render(<App />);
    await renameFromTree("how_lens_works", "how_lens_work");

    const dialog = await screen.findByRole("dialog", { name: "Rename" });
    await userEvent.click(within(dialog).getByRole("button", { name: "Rename only" }));

    await waitFor(() => expect(commandsCalled()).toContain("move-path"));
    expect(commandsCalled()).not.toContain("relink");
  });

  it("does nothing at all when the rename is cancelled", async () => {
    bridge();
    render(<App />);
    await renameFromTree("how_lens_works", "how_lens_work");

    const dialog = await screen.findByRole("dialog", { name: "Rename" });
    await userEvent.click(within(dialog).getByRole("button", { name: "Cancel" }));

    await new Promise((resolve) => setTimeout(resolve, 50));
    expect(commandsCalled()).not.toContain("move-path");
    expect(commandsCalled()).not.toContain("relink");
  });

  it("does not ask when nothing links to the note", async () => {
    // A rename that touches only the file itself is not worth a dialog.
    bridge();
    render(<App />);
    await renameFromTree("test_note", "renamed");

    await waitFor(() => expect(commandsCalled()).toContain("move-path"));
    expect(screen.queryByRole("dialog", { name: "Rename" })).not.toBeInTheDocument();
  });

  it("reports links it could not rewrite, and the report survives a refresh", async () => {
    // A banner would not: `reload` clears it on success, and autosave triggers
    // a reload a second and a half after any edit. This is the one message the
    // user cannot be left to discover by following a dead link.
    bridge(undefined, {
      relink: {
        from: "projects/lens/how_lens_works.md",
        to: "projects/lens/how_lens_work.md",
        dry_run: false,
        updated: [],
        skipped: [
          { path: "projects/lens/profile.md", target: "how_lens_works", reason: "unresolvable" },
        ],
        truncated: EMPTY_TRUNCATION,
      },
    });
    render(<App />);
    await renameFromTree("how_lens_works", "how_lens_work");
    await userEvent.click(
      within(await screen.findByRole("dialog", { name: "Rename" })).getByRole("button", {
        name: "Update links",
      }),
    );

    const report = await screen.findByRole("dialog", { name: /left behind/ });
    expect(within(report).getByText(/could not be rewritten/)).toBeInTheDocument();
    expect(within(report).getByText(/profile\.md/)).toBeInTheDocument();

    // Still there after the refresh that used to wipe it.
    await new Promise((resolve) => setTimeout(resolve, 100));
    expect(screen.getByRole("dialog", { name: /left behind/ })).toBeInTheDocument();
  });

  it("says nothing when every link was carried", async () => {
    bridge(undefined, {
      relink: {
        from: "projects/lens/how_lens_works.md",
        to: "projects/lens/how_lens_work.md",
        dry_run: false,
        updated: [{ path: "projects/lens/profile.md", links: 1, new_revision: "blake3:aa" }],
        skipped: [],
        truncated: EMPTY_TRUNCATION,
      },
    });
    render(<App />);
    await renameFromTree("how_lens_works", "how_lens_work");
    await userEvent.click(
      within(await screen.findByRole("dialog", { name: "Rename" })).getByRole("button", {
        name: "Update links",
      }),
    );

    await waitFor(() => expect(commandsCalled()).toContain("relink"));
    // The user agreed to it and can see the result; a dialog saying it worked
    // is one to dismiss for no information.
    expect(screen.queryByRole("dialog", { name: /left behind/ })).not.toBeInTheDocument();
  });

  it("reports a failed rewrite without pretending the rename failed", async () => {
    // The move landed and the rewrite did not. Rolling the rename back would be
    // a second unchecked write, so both facts are reported instead.
    bridge(undefined, {
      relinkFails: { code: "IO_ERROR", message: "the vault could not be read" },
    });
    render(<App />);
    await renameFromTree("how_lens_works", "how_lens_work");
    await userEvent.click(
      within(await screen.findByRole("dialog", { name: "Rename" })).getByRole("button", {
        name: "Update links",
      }),
    );

    expect(await screen.findByText(/the vault could not be read/)).toBeInTheDocument();
    expect(commandsCalled()).toContain("move-path");
  });

  it("confirms a delete and says where the note goes", async () => {
    bridge();
    render(<App />);

    const menu = await rightClick("test_note");
    await userEvent.click(within(menu).getByRole("menuitem", { name: "Delete…" }));

    const dialog = await screen.findByRole("dialog", { name: "Delete" });
    expect(within(dialog).getByText(/\.trash\//)).toBeInTheDocument();

    await userEvent.click(within(dialog).getByRole("button", { name: "Move to trash" }));

    await waitFor(() => {
      const remove = invoke.mock.calls.find(
        (call) => (call[1] as { command?: string })?.command === "delete-path",
      );
      expect(remove?.[1]).toMatchObject({ request: { path: "ideas/test_note.md" } });
    });
  });

  it("does not delete when the confirmation is dismissed", async () => {
    bridge();
    render(<App />);

    const menu = await rightClick("test_note");
    await userEvent.click(within(menu).getByRole("menuitem", { name: "Delete…" }));

    const dialog = await screen.findByRole("dialog", { name: "Delete" });
    await userEvent.click(within(dialog).getByRole("button", { name: "Cancel" }));

    expect(
      invoke.mock.calls.find((call) => (call[1] as { command?: string })?.command === "delete-path"),
    ).toBeUndefined();
  });
});

describe("a vault that cannot be read", () => {
  it("says why instead of showing an empty workspace forever", async () => {
    // A vault folder that has moved, or a path saved by an older build, would
    // otherwise leave the tree empty and the graph "building" with the reason
    // only in Diagnostics.
    invoke.mockImplementation((command: string) => {
      if (command === "cli_status") return Promise.resolve(STATUS);
      return Promise.resolve({
        ok: false,
        error: { code: "NOT_INITIALIZED", message: "this folder is not a Heimdall vault" },
      });
    });

    render(<App />);

    expect(await screen.findByRole("alert")).toHaveTextContent("NOT_INITIALIZED");
    expect(screen.getByRole("alert")).toHaveTextContent("not a Heimdall vault");
  });

  it("clears the warning once the vault loads", async () => {
    bridge();
    render(<App />);

    await screen.findByRole("region", { name: "Files" });
    expect(screen.queryByRole("alert")).toBeNull();
  });
});

describe("things that should not silently lose work", () => {
  it("stays on the note when the flush before navigating fails", async () => {
    // Replacing the buffer would destroy the edit and leave the reason in a log
    // the user has no reason to open.
    bridge();
    const original = invoke.getMockImplementation()!;
    invoke.mockImplementation((command: string, args: Record<string, unknown>) => {
      const request = args as { command: string };
      if (command === "invoke_cli" && request.command === "write-document") {
        return Promise.resolve({
          ok: false,
          error: { code: "IO_ERROR", message: "the vault is read-only" },
        });
      }
      return original(command, args);
    });

    render(<App />);
    const files = await screen.findByRole("region", { name: "Files" });
    await userEvent.click(within(files).getByText("how_lens_works"));

    const note = screen.getByRole("region", { name: "Note" });
    const properties = await within(note).findByRole("region", { name: "Properties" });
    await userEvent.click(within(properties).getByRole("button", { name: "Remove articles" }));

    await userEvent.click(within(files).getByText("profile"));

    // The edited note is still on screen, and the failure is visible.
    await waitFor(() => expect(titleOf(note)).toBe("how_lens_works"));
    expect(await screen.findByText(/read-only/)).toBeInTheDocument();
  });

  it("stops retrying a write that keeps failing", { timeout: 15_000 }, async () => {
    // A read-only file or an unplugged vault would otherwise spawn a CLI
    // process every second and a half for as long as the window is open.
    bridge();
    const original = invoke.getMockImplementation()!;
    invoke.mockImplementation((command: string, args: Record<string, unknown>) => {
      const request = args as { command: string };
      if (command === "invoke_cli" && request.command === "write-document") {
        return Promise.resolve({ ok: false, error: { code: "IO_ERROR", message: "no" } });
      }
      return original(command, args);
    });

    render(<App />);
    const files = await screen.findByRole("region", { name: "Files" });
    await userEvent.click(within(files).getByText("how_lens_works"));

    const note = screen.getByRole("region", { name: "Note" });
    const properties = await within(note).findByRole("region", { name: "Properties" });
    await userEvent.click(within(properties).getByRole("button", { name: "Remove articles" }));

    const writes = () =>
      invoke.mock.calls.filter(
        (call) => (call[1] as { command?: string })?.command === "write-document",
      ).length;

    // Let autosave make its one attempt.
    await waitFor(() => expect(writes()).toBeGreaterThan(0), { timeout: 4000 });
    const attempted = writes();

    // Then wait out several more autosave windows. This pins the behaviour
    // rather than reproducing a live fault: the serialised save plus the
    // failure backoff both prevent it, and before either existed this climbed
    // by one roughly every second and a half for as long as the window was open.
    await new Promise((resolve) => setTimeout(resolve, AUTOSAVE_WINDOWS));
    expect(writes()).toBe(attempted);
  });
});

describe("a note that cannot be written", () => {
  it("gives an entry a read-only editor rather than a caret that goes nowhere", async () => {
    // Entries are created by an agent and their frontmatter is Heimdall's, so
    // the editor reads them but does not offer to save them. A normal caret
    // would let someone type a paragraph that is silently discarded.
    bridge();
    render(<App />);

    const files = await screen.findByRole("region", { name: "Files" });
    await userEvent.click(within(files).getByText("2026-08-21_23-54-22"));

    const note = screen.getByRole("region", { name: "Note" });
    await waitFor(() => expect(within(note).getByRole("heading", { level: 1 })).toBeInTheDocument());
    await userEvent.click(within(note).getByRole("button", { name: "Edit" }));

    const editor = within(note).getByTestId("source-editor");
    expect(editor.querySelector(".cm-content")).toHaveAttribute("contenteditable", "false");
  });
});

describe("the note's title", () => {
  it("is the filename, and renaming one renames the other", async () => {
    // There is no separate title to keep in step: a note is always called
    // something, and that something is the name of the file.
    bridge();
    render(<App />);

    const files = await screen.findByRole("region", { name: "Files" });
    await userEvent.click(within(files).getByText("test_note"));

    const note = screen.getByRole("region", { name: "Note" });
    // The heading is typed into in source and read in preview.
    expect(within(note).queryByLabelText("Note name")).toBeNull();
    await userEvent.click(await within(note).findByRole("button", { name: "Edit" }));

    const field = await within(note).findByLabelText("Note name");
    expect(field).toHaveValue("test_note");

    await userEvent.clear(field);
    await userEvent.type(field, "renamed_from_title");
    await userEvent.tab();

    await waitFor(() => {
      const move = invoke.mock.calls.find(
        (call) => (call[1] as { command?: string })?.command === "move-path",
      );
      expect(move?.[1]).toMatchObject({
        request: { from: "ideas/test_note.md", to: "ideas/renamed_from_title.md" },
      });
    });
  });

  it("always exists, even for a note with no heading in its body", async () => {
    bridge();
    render(<App />);

    const files = await screen.findByRole("region", { name: "Files" });
    await userEvent.click(within(files).getByText("test_note"));

    const note = screen.getByRole("region", { name: "Note" });
    await userEvent.click(await within(note).findByRole("button", { name: "Edit" }));
    expect(await within(note).findByLabelText("Note name")).toHaveValue("test_note");
  });

  it("is not editable for protected content, which Heimdall names", async () => {
    bridge();
    render(<App />);

    const files = await screen.findByRole("region", { name: "Files" });
    await userEvent.click(within(files).getByText("memory_1"));

    const note = screen.getByRole("region", { name: "Note" });
    await waitFor(() => expect(within(note).getByRole("heading", { level: 1 })).toBeInTheDocument());
    expect(within(note).queryByLabelText("Note name")).toBeNull();

    await userEvent.click(within(note).getByRole("button", { name: "Edit" }));
    expect(within(note).queryByLabelText("Note name")).toBeNull();
  });

  it("shows the heading once, not once as the name and again as a heading", async () => {
    // The heading and the filename are one fact. Drawing the file's own `# `
    // under a copy of it is the same name printed twice.
    bridge("# how_lens_works\n\nBody text.\n");
    render(<App />);

    const files = await screen.findByRole("region", { name: "Files" });
    await userEvent.click(within(files).getByText("how_lens_works"));

    const note = screen.getByRole("region", { name: "Note" });
    await waitFor(() => expect(titleOf(note)).toBe("how_lens_works"));
    expect(within(note).getAllByRole("heading", { level: 1 })).toHaveLength(1);
  });

  it("keeps the heading out of the editor, where it would be the same line twice", async () => {
    bridge("# how_lens_works\n\nBody text.\n");
    render(<App />);

    const files = await screen.findByRole("region", { name: "Files" });
    await userEvent.click(within(files).getByText("how_lens_works"));

    const note = screen.getByRole("region", { name: "Note" });
    await userEvent.click(await within(note).findByRole("button", { name: "Edit" }));

    expect(await within(note).findByLabelText("Note name")).toHaveValue("how_lens_works");
    expect(within(note).getByTestId("source-editor").textContent).not.toContain("# how_lens_works");
  });

  it("abandons an edit on Escape", async () => {
    bridge();
    render(<App />);

    const files = await screen.findByRole("region", { name: "Files" });
    await userEvent.click(within(files).getByText("test_note"));

    const note = screen.getByRole("region", { name: "Note" });
    // The heading is typed into in source and read in preview.
    expect(within(note).queryByLabelText("Note name")).toBeNull();
    await userEvent.click(await within(note).findByRole("button", { name: "Edit" }));

    const field = await within(note).findByLabelText("Note name");
    await userEvent.clear(field);
    await userEvent.type(field, "abandoned{Escape}");

    expect(field).toHaveValue("test_note");
    expect(
      invoke.mock.calls.find((call) => (call[1] as { command?: string })?.command === "move-path"),
    ).toBeUndefined();
  });
});

describe("rules in a note", () => {
  it("draws none under the properties, where the note has none", async () => {
    // A rule that is part of the furniture is a rule nobody can remove, and one
    // the note does not contain.
    bridge("# how_lens_works\n\nBody text.\n");
    render(<App />);

    const files = await screen.findByRole("region", { name: "Files" });
    await userEvent.click(within(files).getByText("how_lens_works"));

    const note = screen.getByRole("region", { name: "Note" });
    await waitFor(() => expect(titleOf(note)).toBe("how_lens_works"));
    expect(note.querySelectorAll("hr")).toHaveLength(0);
  });

  it("draws one where the author wrote one", async () => {
    bridge("# how_lens_works\n\nAbove.\n\n---\n\nBelow.\n");
    render(<App />);

    const files = await screen.findByRole("region", { name: "Files" });
    await userEvent.click(within(files).getByText("how_lens_works"));

    const note = screen.getByRole("region", { name: "Note" });
    await waitFor(() => expect(note.querySelectorAll("hr")).toHaveLength(1));
  });
});

describe("properties that are not on the first line", () => {
  it("renders a block written under the note's heading", async () => {
    // Which is where a note that opens with its name has to put them.
    bridge('# how_lens_works\n\n---\nlinks:\n  - "[[profile]]"\n---\n\nBody text.\n');
    render(<App />);

    const files = await screen.findByRole("region", { name: "Files" });
    await userEvent.click(within(files).getByText("how_lens_works"));

    const note = screen.getByRole("region", { name: "Note" });
    expect(await within(note).findByText("links")).toBeInTheDocument();
    const properties = note.querySelector(".properties") as HTMLElement;
    expect(within(properties).getByRole("button", { name: "profile" })).toBeInTheDocument();
    // The fences are properties, not two rules.
    expect(note.querySelectorAll("hr")).toHaveLength(0);
  });

  it("reads only the first block, because a note has one set of properties", async () => {
    bridge("# how_lens_works\n\n---\nstatus: draft\n---\n\nBody.\n\n---\nlater: block\n---\n");
    render(<App />);

    const files = await screen.findByRole("region", { name: "Files" });
    await userEvent.click(within(files).getByText("how_lens_works"));

    const note = screen.getByRole("region", { name: "Note" });
    expect(await within(note).findByText("status")).toBeInTheDocument();
    expect(within(note).queryByText("later")).toBeNull();
  });
});

describe("dragging a note between folders", () => {
  /**
   * A pointer-event drag, which is what the tree actually uses.
   *
   * HTML5 drag-and-drop does not work inside a WKWebView under Tauri, so
   * firing `dragstart`/`drop` here would test a mechanism the application does
   * not use — and pass while the feature stayed broken.
   */
  async function drag(from: HTMLElement, onto: HTMLElement) {
    // The tree tracks the pointer on the window, so its listeners have to be
    // installed before the events are fired.
    await act(async () => {});

    // `elementFromPoint` is what the tree reads the drop target off, and jsdom
    // has no layout, so it is pointed at the row under test.
    const original = document.elementFromPoint;
    document.elementFromPoint = () => onto;
    try {
      fireEvent.pointerDown(from, { button: 0, pointerId: 1, clientX: 0, clientY: 0 });
      fireEvent.pointerMove(window, { pointerId: 1, clientX: 0, clientY: 40 });
      fireEvent.pointerUp(window, { pointerId: 1, clientX: 0, clientY: 40 });
    } finally {
      document.elementFromPoint = original;
    }
  }

  const row = (files: HTMLElement, name: string) =>
    within(files).getByText(name).closest<HTMLElement>('[role="treeitem"]')!;

  it("moves a note onto the folder it is dropped on", async () => {
    bridge();
    render(<App />);

    const files = await screen.findByRole("region", { name: "Files" });
    await drag(row(files, "test_note"), row(files, "projects"));

    await waitFor(() => {
      const move = invoke.mock.calls.find(
        (call) => (call[1] as { command?: string })?.command === "move-path",
      );
      expect(move?.[1]).toMatchObject({
        request: { from: "ideas/test_note.md", to: "projects/test_note.md" },
      });
    });
  });

  it("drops into the folder a note lives in when dropped on that note", async () => {
    bridge();
    render(<App />);

    const files = await screen.findByRole("region", { name: "Files" });
    await drag(row(files, "test_note"), row(files, "my_project"));

    await waitFor(() => {
      const move = invoke.mock.calls.find(
        (call) => (call[1] as { command?: string })?.command === "move-path",
      );
      expect(move?.[1]).toMatchObject({
        request: { from: "ideas/test_note.md", to: "projects/test_note.md" },
      });
    });
  });

  it("refuses to drop into the protected tree, which move-path would reject", async () => {
    bridge();
    render(<App />);

    const files = await screen.findByRole("region", { name: "Files" });
    await drag(row(files, "test_note"), row(files, "memories"));

    await new Promise((resolve) => setTimeout(resolve, 50));
    expect(
      invoke.mock.calls.find((call) => (call[1] as { command?: string })?.command === "move-path"),
    ).toBeUndefined();
  });

  it("does nothing when a note is dropped back where it started", async () => {
    bridge();
    render(<App />);

    const files = await screen.findByRole("region", { name: "Files" });
    await drag(row(files, "test_note"), row(files, "ideas"));

    await new Promise((resolve) => setTimeout(resolve, 50));
    expect(
      invoke.mock.calls.find((call) => (call[1] as { command?: string })?.command === "move-path"),
    ).toBeUndefined();
  });

  it("asks before a drag that would break links, too", async () => {
    // A drag changes a path just as a rename does, so it breaks the same links.
    bridge();
    render(<App />);

    const files = await screen.findByRole("region", { name: "Files" });
    await drag(row(files, "how_lens_works"), row(files, "ideas"));

    const dialog = await screen.findByRole("dialog", { name: "Rename" });
    expect(within(dialog).getByText(/2 notes link here/)).toBeInTheDocument();
    expect(commandsCalled()).not.toContain("move-path");
  });

  it("opens a note on a click, and not on a drag", async () => {
    bridge();
    render(<App />);

    const files = await screen.findByRole("region", { name: "Files" });
    const note = screen.getByRole("region", { name: "Note" });

    // A press that travels is a drag: it must not also open the note.
    await drag(row(files, "test_note"), row(files, "projects"));
    fireEvent.click(row(files, "test_note"));
    await new Promise((resolve) => setTimeout(resolve, 50));
    expect(titleOf(note)).toBe("");

    // A press that does not travel is a click.
    await userEvent.click(within(files).getByText("hello_world"));
    await waitFor(() => expect(titleOf(note)).toBe("hello_world"));
  });
});

describe("a vault that cannot be read", () => {
  it("says why instead of showing an empty workspace forever", async () => {
    // A vault folder that has moved, or a path saved by an older build, would
    // otherwise leave the tree empty and the graph "building" with the reason
    // only in Diagnostics.
    invoke.mockImplementation((command: string) => {
      if (command === "cli_status") return Promise.resolve(STATUS);
      return Promise.resolve({
        ok: false,
        error: { code: "NOT_INITIALIZED", message: "this folder is not a Heimdall vault" },
      });
    });

    render(<App />);

    expect(await screen.findByRole("alert")).toHaveTextContent("NOT_INITIALIZED");
    expect(screen.getByRole("alert")).toHaveTextContent("not a Heimdall vault");
  });

  it("clears the warning once the vault loads", async () => {
    bridge();
    render(<App />);

    await screen.findByRole("region", { name: "Files" });
    expect(screen.queryByRole("alert")).toBeNull();
  });
});

describe("things that should not silently lose work", () => {
  it("stays on the note when the flush before navigating fails", async () => {
    // Replacing the buffer would destroy the edit and leave the reason in a log
    // the user has no reason to open.
    bridge();
    const original = invoke.getMockImplementation()!;
    invoke.mockImplementation((command: string, args: Record<string, unknown>) => {
      const request = args as { command: string };
      if (command === "invoke_cli" && request.command === "write-document") {
        return Promise.resolve({
          ok: false,
          error: { code: "IO_ERROR", message: "the vault is read-only" },
        });
      }
      return original(command, args);
    });

    render(<App />);
    const files = await screen.findByRole("region", { name: "Files" });
    await userEvent.click(within(files).getByText("how_lens_works"));

    const note = screen.getByRole("region", { name: "Note" });
    const properties = await within(note).findByRole("region", { name: "Properties" });
    await userEvent.click(within(properties).getByRole("button", { name: "Remove articles" }));

    await userEvent.click(within(files).getByText("profile"));

    // The edited note is still on screen, and the failure is visible.
    await waitFor(() => expect(titleOf(note)).toBe("how_lens_works"));
    expect(await screen.findByText(/read-only/)).toBeInTheDocument();
  });

  it("stops retrying a write that keeps failing", { timeout: 15_000 }, async () => {
    // A read-only file or an unplugged vault would otherwise spawn a CLI
    // process every second and a half for as long as the window is open.
    bridge();
    const original = invoke.getMockImplementation()!;
    invoke.mockImplementation((command: string, args: Record<string, unknown>) => {
      const request = args as { command: string };
      if (command === "invoke_cli" && request.command === "write-document") {
        return Promise.resolve({ ok: false, error: { code: "IO_ERROR", message: "no" } });
      }
      return original(command, args);
    });

    render(<App />);
    const files = await screen.findByRole("region", { name: "Files" });
    await userEvent.click(within(files).getByText("how_lens_works"));

    const note = screen.getByRole("region", { name: "Note" });
    const properties = await within(note).findByRole("region", { name: "Properties" });
    await userEvent.click(within(properties).getByRole("button", { name: "Remove articles" }));

    const writes = () =>
      invoke.mock.calls.filter(
        (call) => (call[1] as { command?: string })?.command === "write-document",
      ).length;

    // Let autosave make its one attempt.
    await waitFor(() => expect(writes()).toBeGreaterThan(0), { timeout: 4000 });
    const attempted = writes();

    // Then wait out several more autosave windows. This pins the behaviour
    // rather than reproducing a live fault: the serialised save plus the
    // failure backoff both prevent it, and before either existed this climbed
    // by one roughly every second and a half for as long as the window was open.
    await new Promise((resolve) => setTimeout(resolve, AUTOSAVE_WINDOWS));
    expect(writes()).toBe(attempted);
  });
});

describe("a note that cannot be written", () => {
  it("gives an entry a read-only editor rather than a caret that goes nowhere", async () => {
    // Entries are created by an agent and their frontmatter is Heimdall's, so
    // the editor reads them but does not offer to save them. A normal caret
    // would let someone type a paragraph that is silently discarded.
    bridge();
    render(<App />);

    const files = await screen.findByRole("region", { name: "Files" });
    await userEvent.click(within(files).getByText("2026-08-21_23-54-22"));

    const note = screen.getByRole("region", { name: "Note" });
    await waitFor(() => expect(within(note).getByRole("heading", { level: 1 })).toBeInTheDocument());
    await userEvent.click(within(note).getByRole("button", { name: "Edit" }));

    const editor = within(note).getByTestId("source-editor");
    expect(editor.querySelector(".cm-content")).toHaveAttribute("contenteditable", "false");
  });
});

describe("the note's title", () => {
  it("is the filename, and renaming one renames the other", async () => {
    // There is no separate title to keep in step: a note is always called
    // something, and that something is the name of the file.
    bridge();
    render(<App />);

    const files = await screen.findByRole("region", { name: "Files" });
    await userEvent.click(within(files).getByText("test_note"));

    const note = screen.getByRole("region", { name: "Note" });
    // The heading is typed into in source and read in preview.
    expect(within(note).queryByLabelText("Note name")).toBeNull();
    await userEvent.click(await within(note).findByRole("button", { name: "Edit" }));

    const field = await within(note).findByLabelText("Note name");
    expect(field).toHaveValue("test_note");

    await userEvent.clear(field);
    await userEvent.type(field, "renamed_from_title");
    await userEvent.tab();

    await waitFor(() => {
      const move = invoke.mock.calls.find(
        (call) => (call[1] as { command?: string })?.command === "move-path",
      );
      expect(move?.[1]).toMatchObject({
        request: { from: "ideas/test_note.md", to: "ideas/renamed_from_title.md" },
      });
    });
  });

  it("always exists, even for a note with no heading in its body", async () => {
    bridge();
    render(<App />);

    const files = await screen.findByRole("region", { name: "Files" });
    await userEvent.click(within(files).getByText("test_note"));

    const note = screen.getByRole("region", { name: "Note" });
    await userEvent.click(await within(note).findByRole("button", { name: "Edit" }));
    expect(await within(note).findByLabelText("Note name")).toHaveValue("test_note");
  });

  it("is not editable for protected content, which Heimdall names", async () => {
    bridge();
    render(<App />);

    const files = await screen.findByRole("region", { name: "Files" });
    await userEvent.click(within(files).getByText("memory_1"));

    const note = screen.getByRole("region", { name: "Note" });
    await waitFor(() => expect(within(note).getByRole("heading", { level: 1 })).toBeInTheDocument());
    expect(within(note).queryByLabelText("Note name")).toBeNull();

    await userEvent.click(within(note).getByRole("button", { name: "Edit" }));
    expect(within(note).queryByLabelText("Note name")).toBeNull();
  });

  it("abandons an edit on Escape", async () => {
    bridge();
    render(<App />);

    const files = await screen.findByRole("region", { name: "Files" });
    await userEvent.click(within(files).getByText("test_note"));

    const note = screen.getByRole("region", { name: "Note" });
    // The heading is typed into in source and read in preview.
    expect(within(note).queryByLabelText("Note name")).toBeNull();
    await userEvent.click(await within(note).findByRole("button", { name: "Edit" }));

    const field = await within(note).findByLabelText("Note name");
    await userEvent.clear(field);
    await userEvent.type(field, "abandoned{Escape}");

    expect(field).toHaveValue("test_note");
    expect(
      invoke.mock.calls.find((call) => (call[1] as { command?: string })?.command === "move-path"),
    ).toBeUndefined();
  });
});


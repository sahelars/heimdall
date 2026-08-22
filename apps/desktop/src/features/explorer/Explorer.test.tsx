/**
 * The file pane's behaviour (SPEC §17).
 */

import { act, fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { Explorer } from "./Explorer";
import type { TreeInput } from "./tree";

const ENTRIES: TreeInput[] = [
  { path: "projects", kind: "directory" },
  { path: "projects/lens", kind: "directory" },
  { path: "projects/lens/how_lens_works.md", kind: "document" },
  { path: "ideas", kind: "directory" },
  { path: "ideas/hello_world.md", kind: "document" },
];

function show(openPath: string | null = null) {
  const onOpen = vi.fn();
  const view = render(
    <Explorer
      entries={ENTRIES}
      openPath={openPath}
      onOpen={onOpen}
      onNewNote={vi.fn()}
      onNewFolder={vi.fn()}
      onContextMenu={vi.fn()}
      onMove={vi.fn()}
    />,
  );
  return { ...view, onOpen };
}

describe("the tree", () => {
  it("shows the vault with its folders open", () => {
    show();
    expect(screen.getByText("how_lens_works")).toBeInTheDocument();
    expect(screen.getByText("hello_world")).toBeInTheDocument();
  });

  it("opens a note when its row is clicked", async () => {
    const { onOpen } = show();
    await userEvent.click(screen.getByText("hello_world"));
    expect(onOpen).toHaveBeenCalledWith("ideas/hello_world.md");
  });

  it("collapses and expands every folder at once", async () => {
    show();
    await userEvent.click(screen.getByRole("button", { name: "Collapse all" }));
    expect(screen.queryByText("how_lens_works")).toBeNull();

    await userEvent.click(screen.getByRole("button", { name: "Expand all" }));
    expect(screen.getByText("how_lens_works")).toBeInTheDocument();
  });

  it("reveals a note opened from somewhere else", async () => {
    // Reached from the quick switcher, a wikilink, a backlink or the graph —
    // and highlighting a row inside a closed folder says nothing about where
    // you are.
    const { rerender, onOpen } = show();
    await userEvent.click(screen.getByRole("button", { name: "Collapse all" }));
    expect(screen.queryByText("how_lens_works")).toBeNull();

    rerender(
      <Explorer
        entries={ENTRIES}
        openPath="projects/lens/how_lens_works.md"
        onOpen={onOpen}
        onNewNote={vi.fn()}
        onNewFolder={vi.fn()}
        onContextMenu={vi.fn()}
      onMove={vi.fn()}
      />,
    );

    const row = await screen.findByText("how_lens_works");
    expect(row).toBeInTheDocument();
    expect(row.closest('[role="treeitem"]')).toHaveAttribute("aria-current", "page");
  });

  it("leaves folders the user closed alone when they hold nothing relevant", async () => {
    const { rerender, onOpen } = show();
    await userEvent.click(screen.getByRole("button", { name: "Collapse all" }));

    rerender(
      <Explorer
        entries={ENTRIES}
        openPath="ideas/hello_world.md"
        onOpen={onOpen}
        onNewNote={vi.fn()}
        onNewFolder={vi.fn()}
        onContextMenu={vi.fn()}
      onMove={vi.fn()}
      />,
    );

    expect(await screen.findByText("hello_world")).toBeInTheDocument();
    // `projects` was not on the way, so it stays shut.
    expect(screen.queryByText("how_lens_works")).toBeNull();
  });

  it("reverses the order without lifting files above folders", async () => {
    show();
    await userEvent.click(screen.getByRole("button", { name: "Change sort order" }));

    const rows = screen.getAllByRole("treeitem").map((row) => row.textContent);
    expect(rows[0]).toBe("projects");
  });

  it("says so when the listing stopped early", () => {
    render(
      <Explorer
        entries={ENTRIES}
        openPath={null}
        truncated
        onOpen={vi.fn()}
        onNewNote={vi.fn()}
        onNewFolder={vi.fn()}
        onContextMenu={vi.fn()}
      onMove={vi.fn()}
      />,
    );
    expect(screen.getByText(/stopped early/)).toBeInTheDocument();
  });
});

describe("dropping onto a note at the vault root", () => {
  it("lands in the root, not in a folder named after the truncated note", async () => {
    // `path.slice(0, path.lastIndexOf("/"))` returns -1 for a root-level note
    // and quietly drops the last character, so a drop near `Sam.md` tried to
    // move into a folder called "Sam.m".
    const onMove = vi.fn();
    const entries: TreeInput[] = [
      { path: "Sam.md", kind: "document" },
      { path: "ideas", kind: "directory" },
      { path: "ideas/hello_world.md", kind: "document" },
    ];

    render(
      <Explorer
        entries={entries}
        openPath={null}
        onOpen={vi.fn()}
        onNewNote={vi.fn()}
        onNewFolder={vi.fn()}
        onContextMenu={vi.fn()}
        onMove={onMove}
      />,
    );

    const rows = screen.getAllByRole("treeitem");
    const from = rows.find((row) => row.dataset.path === "ideas/hello_world.md")!;
    const onto = rows.find((row) => row.dataset.path === "Sam.md")!;

    // The tree tracks the pointer on the window, so its listeners have to be
    // installed before the events are fired.
    await act(async () => {});

    const original = document.elementFromPoint;
    document.elementFromPoint = () => onto;
    try {
      fireEvent.pointerDown(from, { button: 0, pointerId: 1, clientX: 0, clientY: 0 });
      fireEvent.pointerMove(window, { pointerId: 1, clientX: 0, clientY: 40 });
      fireEvent.pointerUp(window, { pointerId: 1, clientX: 0, clientY: 40 });
    } finally {
      document.elementFromPoint = original;
    }

    expect(onMove).toHaveBeenCalledWith("ideas/hello_world.md", "");
  });
});

describe("dragging a note out of a folder", () => {
  it("lands in the vault root when it is dropped on the tree itself", async () => {
    // The empty space under the last row is the root as a drop target. Without
    // it the drag reported "nowhere to drop" over most of the pane, and the
    // only way out of a folder was a rename.
    const onMove = vi.fn();

    render(
      <Explorer
        entries={ENTRIES}
        openPath={null}
        onOpen={vi.fn()}
        onNewNote={vi.fn()}
        onNewFolder={vi.fn()}
        onContextMenu={vi.fn()}
        onMove={onMove}
      />,
    );

    const from = screen
      .getAllByRole("treeitem")
      .find((row) => row.dataset.path === "ideas/hello_world.md")!;
    const tree = screen.getByRole("tree");

    await act(async () => {});

    const original = document.elementFromPoint;
    document.elementFromPoint = () => tree;
    try {
      fireEvent.pointerDown(from, { button: 0, pointerId: 1, clientX: 0, clientY: 0 });
      fireEvent.pointerMove(window, { pointerId: 1, clientX: 0, clientY: 300 });
      fireEvent.pointerUp(window, { pointerId: 1, clientX: 0, clientY: 300 });
    } finally {
      document.elementFromPoint = original;
    }

    expect(onMove).toHaveBeenCalledWith("ideas/hello_world.md", "");
  });

  it("refuses to drop a folder into itself", async () => {
    const onMove = vi.fn();

    render(
      <Explorer
        entries={ENTRIES}
        openPath={null}
        onOpen={vi.fn()}
        onNewNote={vi.fn()}
        onNewFolder={vi.fn()}
        onContextMenu={vi.fn()}
        onMove={onMove}
      />,
    );

    const rows = screen.getAllByRole("treeitem");
    const from = rows.find((row) => row.dataset.path === "projects")!;
    const onto = rows.find((row) => row.dataset.path === "projects/lens")!;

    await act(async () => {});

    const original = document.elementFromPoint;
    document.elementFromPoint = () => onto;
    try {
      fireEvent.pointerDown(from, { button: 0, pointerId: 1, clientX: 0, clientY: 0 });
      fireEvent.pointerMove(window, { pointerId: 1, clientX: 0, clientY: 40 });
      expect(screen.getByText(/nowhere to drop/)).toBeInTheDocument();
      fireEvent.pointerUp(window, { pointerId: 1, clientX: 0, clientY: 40 });
    } finally {
      document.elementFromPoint = original;
    }

    expect(onMove).not.toHaveBeenCalled();
  });
});

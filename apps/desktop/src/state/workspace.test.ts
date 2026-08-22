import { describe, expect, it } from "vitest";

import {
  canGoBack,
  canGoForward,
  currentPath,
  EMPTY_HISTORY,
  forget,
  goBack,
  goForward,
  isDirty,
  shouldAutosave,
  visit,
  type Buffer,
} from "./workspace";

/** Walk a list of paths through the history. */
function opened(...paths: string[]) {
  return paths.reduce(visit, EMPTY_HISTORY);
}

describe("history", () => {
  it("moves back and forward through what was opened", () => {
    let history = opened("a.md", "b.md", "c.md");
    expect(currentPath(history)).toBe("c.md");

    history = goBack(history);
    expect(currentPath(history)).toBe("b.md");
    expect(canGoForward(history)).toBe(true);

    history = goForward(history);
    expect(currentPath(history)).toBe("c.md");
    expect(canGoForward(history)).toBe(false);
  });

  it("does not record opening the note already showing", () => {
    const history = opened("a.md", "a.md", "a.md");
    expect(history.entries).toEqual(["a.md"]);
  });

  it("drops the forward branch when a new note is opened after going back", () => {
    let history = opened("a.md", "b.md", "c.md");
    history = goBack(history);
    history = visit(history, "d.md");

    expect(history.entries).toEqual(["a.md", "b.md", "d.md"]);
    expect(canGoForward(history)).toBe(false);
  });

  it("stays put at either end rather than running off it", () => {
    const start = opened("a.md");
    expect(canGoBack(start)).toBe(false);
    expect(goBack(start)).toBe(start);
    expect(goForward(start)).toBe(start);
  });

  it("keeps only the most recent entries", () => {
    const many = Array.from({ length: 80 }, (_, index) => `note_${index}.md`);
    const history = opened(...many);

    expect(history.entries).toHaveLength(50);
    expect(currentPath(history)).toBe("note_79.md");
  });

  it("forgets a note that was deleted, so the back arrow cannot reach it", () => {
    let history = opened("a.md", "gone.md", "b.md");
    history = forget(history, "gone.md");

    expect(history.entries).toEqual(["a.md", "b.md"]);
    expect(currentPath(history)).toBe("b.md");
  });

  it("leaves the history alone when the deleted note was never in it", () => {
    const history = opened("a.md");
    expect(forget(history, "other.md")).toBe(history);
  });
});

function buffer(overrides: Partial<Buffer> = {}): Buffer {
  return {
    path: "ideas/note.md",
    disk: "one\n",
    buffer: "one\n",
    revision: "blake3:aa",
    editable: true,
    saving: false,
    conflict: null,
    ...overrides,
  };
}

describe("dirty tracking", () => {
  it("compares the text rather than trusting a flag", () => {
    expect(isDirty(buffer())).toBe(false);
    expect(isDirty(buffer({ buffer: "two\n" }))).toBe(true);
    // Typing and undoing back to the original is not a change.
    expect(isDirty(buffer({ buffer: "one\n", disk: "one\n" }))).toBe(false);
  });
});

describe("autosaving", () => {
  it("runs for an edited, editable note", () => {
    expect(shouldAutosave(buffer({ buffer: "two\n" }))).toBe(true);
  });

  it("does not run for a note that cannot be saved from here", () => {
    expect(shouldAutosave(buffer({ buffer: "two\n", editable: false }))).toBe(false);
  });

  it("does not stack a second save on top of one in flight", () => {
    expect(shouldAutosave(buffer({ buffer: "two\n", saving: true }))).toBe(false);
  });

  it("stops while a conflict is unresolved", () => {
    // Otherwise every autosave fails on the same stale revision and the note
    // sits in a loop nobody can read past.
    expect(
      shouldAutosave(buffer({ buffer: "two\n", conflict: { theirRevision: "blake3:bb" } })),
    ).toBe(false);
  });
});

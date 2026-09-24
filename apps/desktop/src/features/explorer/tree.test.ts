/**
 * The sidebar's tree (SPEC §17).
 *
 * The reference screenshots are mostly a statement about this shape: folders
 * nested the way their paths say, and folders always above files.
 */

import { describe, expect, it } from "vitest";

import {
  ancestorsOf,
  buildTree,
  containsLocked,
  folderPaths,
  lockedFolders,
  type TreeInput,
} from "./tree";

const doc = (path: string): TreeInput => ({ path, kind: "document" });
const dir = (path: string): TreeInput => ({ path, kind: "directory" });

/** A compact rendering of the tree, for readable assertions. */
function outline(nodes: ReturnType<typeof buildTree>, depth = 0): string[] {
  return nodes.flatMap((node) => [
    `${"  ".repeat(depth)}${node.label}${node.kind === "directory" ? "/" : ""}`,
    ...outline(node.children, depth + 1),
  ]);
}

describe("building the tree", () => {
  it("nests notes under the folders their paths imply", () => {
    const tree = buildTree([
      doc("projects/lens/how_lens_works.md"),
      doc("projects/lens/profile.md"),
      doc("ideas/hello_world.md"),
    ]);

    expect(outline(tree)).toEqual([
      "ideas/",
      "  hello_world",
      "projects/",
      "  lens/",
      "    how_lens_works",
      "    profile",
    ]);
  });

  it("infers a folder implied by a note deeper than the listing reached", () => {
    const tree = buildTree([dir("a"), doc("a/b/c/note.md")]);

    expect(outline(tree)).toEqual(["a/", "  b/", "    c/", "      note"]);
  });

  it("carries each entry's lock state onto its node", () => {
    const tree = buildTree([
      { path: "private", kind: "directory", locked: true },
      { path: "private/plan.md", kind: "document", locked: true },
      { path: "private/open.md", kind: "document", locked: false },
      doc("ideas/note.md"),
    ]);

    const locked = tree.find((node) => node.path === "private")!;
    expect(locked.locked).toBe(true);
    expect(locked.children.map((child) => [child.label, child.locked])).toEqual([
      ["open", false],
      ["plan", true],
    ]);
    expect(tree.find((node) => node.path === "ideas")!.locked).toBe(false);
  });

  it("lets an inferred folder inherit its parent's lock", () => {
    const tree = buildTree([
      { path: "private", kind: "directory", locked: true },
      { path: "private/deep/note.md", kind: "document", locked: true },
    ]);

    expect(tree[0]!.children[0]!.locked).toBe(true);
  });

  it("finds locked folders, and folders holding anything locked", () => {
    const tree = buildTree([
      { path: "open", kind: "directory", locked: false },
      { path: "open/pinned.md", kind: "document", locked: true },
      { path: "private", kind: "directory", locked: true },
      doc("free.md"),
    ]);

    expect([...lockedFolders(tree)]).toEqual(["private"]);
    expect(containsLocked(tree.find((node) => node.path === "open")!)).toBe(true);
    expect(containsLocked(tree.find((node) => node.path === "free.md")!)).toBe(false);
  });

  it("keeps a folder that holds nothing yet", () => {
    const tree = buildTree([dir("projects/empty"), doc("projects/note.md")]);

    expect(outline(tree)).toEqual(["projects/", "  empty/", "  note"]);
  });

  it("puts folders above files and orders each group by name", () => {
    const tree = buildTree([doc("z_note.md"), doc("a_note.md"), dir("m_folder")]);

    expect(outline(tree)).toEqual(["m_folder/", "a_note", "z_note"]);
  });

  it("reverses within each group when asked, without lifting files above folders", () => {
    const tree = buildTree([doc("a.md"), doc("b.md"), dir("x"), dir("y")], "name-desc");

    expect(outline(tree)).toEqual(["y/", "x/", "b", "a"]);
  });

  it("shows a note by its name rather than its filename", () => {
    const tree = buildTree([doc("ideas/hello_world.md")]);
    expect(tree[0]!.children[0]!.label).toBe("hello_world");
  });

  it("handles a note at the vault root", () => {
    const tree = buildTree([doc("README.md")]);
    expect(outline(tree)).toEqual(["README"]);
  });

  it("does not duplicate a folder that is both listed and implied", () => {
    const tree = buildTree([dir("projects"), doc("projects/note.md"), dir("projects")]);
    expect(tree.filter((node) => node.path === "projects")).toHaveLength(1);
  });
});

describe("navigating the tree", () => {
  it("names every folder, for collapse-all", () => {
    const tree = buildTree([doc("a/b/c/note.md"), doc("d/note.md")]);
    expect(folderPaths(tree).sort()).toEqual(["a", "a/b", "a/b/c", "d"]);
  });

  it("names the folders a note needs open to be visible", () => {
    expect(ancestorsOf("projects/lens/how_lens_works.md")).toEqual(["projects", "projects/lens"]);
    expect(ancestorsOf("README.md")).toEqual([]);
  });
});

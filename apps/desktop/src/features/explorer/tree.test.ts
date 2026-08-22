/**
 * The sidebar's tree (SPEC §17).
 *
 * The reference screenshots are mostly a statement about this shape: `aios/`
 * with its memories and entries nested beneath it, ordinary folders below, and
 * folders always above files.
 */

import { describe, expect, it } from "vitest";

import { ancestorsOf, buildTree, folderPaths, type TreeInput } from "./tree";

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

  it("infers the folders of the protected tree, which the index reports as notes only", () => {
    // `list-documents` never returns anything under `aios/`, so those folders
    // exist only because the note paths imply them.
    const tree = buildTree([
      doc("aios/AGENTS.md"),
      doc("aios/memories/memory.md"),
      doc("aios/memories/extended/memory_1.md"),
      doc("aios/conversations/2026-08-16_10-30-00.md"),
    ]);

    expect(outline(tree)).toEqual([
      "aios/",
      "  conversations/",
      "    2026-08-16_10-30-00",
      "  memories/",
      "    extended/",
      "      memory_1",
      // `memory.md` sits inside `memories/`; `AGENTS.md` sits beside it, one
      // level up — exactly the nesting the reference screenshots show.
      "    memory",
      "  AGENTS",
    ]);
  });

  it("marks everything under the protected tree, and nothing else", () => {
    const tree = buildTree([doc("aios/memories/memory.md"), doc("ideas/note.md")]);

    const aios = tree.find((node) => node.path === "aios")!;
    expect(aios.inAios).toBe(true);
    expect(aios.children[0]!.children[0]!.inAios).toBe(true);
    expect(tree.find((node) => node.path === "ideas")!.inAios).toBe(false);
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

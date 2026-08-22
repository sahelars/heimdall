import { describe, expect, it } from "vitest";

import { score, search } from "./fuzzy";

describe("scoring", () => {
  it("matches a subsequence rather than only a prefix", () => {
    expect(score("hlw", "projects/lens/how_lens_works.md")).not.toBeNull();
  });

  it("rejects a query whose letters are not all there in order", () => {
    expect(score("zzz", "ideas/note.md")).toBeNull();
    expect(score("wl", "projects/lens/works_later.md")).not.toBeNull();
    expect(score("lw", "abc.md")).toBeNull();
  });

  it("reports where it matched, so the result can be highlighted", () => {
    const match = score("no", "ideas/note.md")!;
    expect(match.positions).toEqual([6, 7]);
  });

  it("ignores case and spaces in the query", () => {
    expect(score("HOW LENS", "projects/how_lens.md")).not.toBeNull();
  });

  it("matches everything on an empty query", () => {
    expect(score("", "anything.md")?.positions).toEqual([]);
  });
});

describe("ranking", () => {
  it("prefers a match in the note's name over one in its folders", () => {
    const results = search("lens", ["lens/other_note.md", "projects/lens.md"]);
    expect(results[0]!.path).toBe("projects/lens.md");
  });

  it("prefers a run of consecutive letters over scattered ones", () => {
    const results = search("note", ["n_o_t_e_x.md", "note.md"]);
    expect(results[0]!.path).toBe("note.md");
  });

  it("prefers the shorter path when two match equally well", () => {
    const results = search("note", ["note.md", "a/very/deep/folder/note.md"]);
    expect(results[0]!.path).toBe("note.md");
  });

  it("prefers a match at a word boundary", () => {
    const results = search("w", ["how_works.md", "howw.md"]);
    expect(results[0]!.path).toBe("how_works.md");
  });

  it("returns nothing rather than everything when nothing matches", () => {
    expect(search("qqqq", ["a.md", "b.md"])).toEqual([]);
  });

  it("caps how many results it returns", () => {
    const many = Array.from({ length: 200 }, (_, index) => `note_${index}.md`);
    expect(search("note", many, 10)).toHaveLength(10);
  });
});

import { describe, expect, it } from "vitest";

import { parseWikilink, resolveWikilink, splitWikilinks } from "./wikilinks";

describe("parsing a wikilink", () => {
  it("reads the plain form", () => {
    expect(parseWikilink("profile")).toEqual({ target: "profile", label: "profile" });
  });

  it("prefers an alias for the label", () => {
    expect(parseWikilink("profile|My profile")).toEqual({
      target: "profile",
      label: "My profile",
    });
  });

  it("keeps the heading out of the target but visible in the label", () => {
    expect(parseWikilink("profile#Setup")).toEqual({
      target: "profile",
      label: "profile › Setup",
    });
  });

  it("has nothing to point at for a same-note heading link", () => {
    expect(parseWikilink("#Setup")).toBeNull();
  });
});

describe("splitting text", () => {
  it("keeps the surrounding prose in order", () => {
    expect(splitWikilinks("See [[profile]] then [[articles]].")).toEqual([
      "See ",
      { target: "profile", label: "profile" },
      " then ",
      { target: "articles", label: "articles" },
      ".",
    ]);
  });

  it("leaves an unterminated bracket as text", () => {
    expect(splitWikilinks("a [[broken")).toEqual(["a [[broken"]);
  });

  it("passes through text with no links at all", () => {
    expect(splitWikilinks("plain")).toEqual(["plain"]);
  });
});

describe("resolving a target", () => {
  const paths = ["target.md", "a/b/c/target.md", "projects/lens/profile.md", "ideas/note.md"];

  it("matches a full vault path", () => {
    expect(resolveWikilink("a/b/c/target", "ideas/note.md", paths)).toBe("a/b/c/target.md");
  });

  it("prefers the shallowest note when only a name is given", () => {
    expect(resolveWikilink("target", "ideas/note.md", paths)).toBe("target.md");
  });

  it("reads a bare name as vault-relative first, matching the Rust index", () => {
    // `[[target]]` names a note across the whole vault, so a `target.md` at the
    // root wins over one beside the linking note. The graph resolves the same
    // way; if these two disagreed, a link would land somewhere the graph did
    // not draw an edge to.
    const near = ["projects/lens/target.md", "target.md"];
    expect(resolveWikilink("target", "projects/lens/source.md", near)).toBe("target.md");
  });

  it("falls back to the nearest note when nothing matches at the root", () => {
    const near = ["projects/lens/target.md", "a/b/c/target.md"];
    expect(resolveWikilink("target", "projects/lens/source.md", near)).toBe(
      "projects/lens/target.md",
    );
  });

  it("accepts a target that already names the extension", () => {
    expect(resolveWikilink("ideas/note.md", "target.md", paths)).toBe("ideas/note.md");
  });

  it("reports nothing rather than guessing", () => {
    expect(resolveWikilink("nowhere", "ideas/note.md", paths)).toBeNull();
  });

  it("resolves regardless of case, the way the vault contract does", () => {
    expect(resolveWikilink("TARGET", "ideas/note.md", paths)).toBe("target.md");
  });
});

/**
 * Frontmatter round-tripping (SPEC §17).
 *
 * The Properties panel edits one key of a block that agents and other editors also
 * write. Anything it does not touch has to come back byte for byte, or every
 * property edit becomes a diff across the whole block.
 */

import { describe, expect, it } from "vitest";

import {
  addProperty,
  joinTitle,
  parseFrontmatter,
  readProperties,
  removeProperty,
  removeValue,
  serializeFrontmatter,
  splitNote,
  withFrontmatter,
  withTitle,
  withoutTitle,
} from "./frontmatter";

const NOTE = `---
links:
  - "[[profile]]"
  - "[[articles]]"
---

### Create Account Pipeline
`;

/** What a note written in this editor looks like: heading first, block under it. */
const TITLED = `# Create Account Pipeline

---
links:
  - "[[profile]]"
---

Body text.
`;

describe("splitting a note", () => {
  it("separates the block from the body", () => {
    const split = splitNote(NOTE);
    expect(split.frontmatter).toBe('links:\n  - "[[profile]]"\n  - "[[articles]]"');
    expect(split.body).toBe("\n### Create Account Pipeline\n");
  });

  it("finds the block under the title, where a titled note puts it", () => {
    // The whole point: a note opens with its name, so its properties cannot be
    // on the first line, and a parser that insists on that finds nothing.
    const split = splitNote(TITLED);
    expect(split.title).toBe("Create Account Pipeline");
    expect(split.frontmatter).toBe('links:\n  - "[[profile]]"');
    expect(split.body).toBe("\n\nBody text.\n");
  });

  it("takes only the note's first line of content as its title", () => {
    // A heading further down is a section. Hoisting it into the header because
    // the note happens to open without one is a surprise nobody could undo.
    expect(splitNote(NOTE).title).toBeNull();
    expect(splitNote("Some prose\n\n# Later\n").title).toBeNull();
    expect(splitNote("# Just a note\n").title).toBe("Just a note");
  });

  it("takes the title from under a block written above it", () => {
    expect(splitNote("---\ntags: [a]\n---\n\n# Late\n\nBody.\n").title).toBe("Late");
  });

  it("treats a note with no block as all body", () => {
    expect(splitNote("# Just a note\n")).toEqual({
      title: "Just a note",
      frontmatter: null,
      body: "",
    });
  });

  it("does not read an unterminated opening rule as frontmatter", () => {
    // The same rule heimdall-core applies on the way in: a note whose body opens
    // with a horizontal rule is a note, not a broken YAML block.
    const text = "---\nnot really yaml\n\nstill the body\n";
    expect(splitNote(text).frontmatter).toBeNull();
  });

  it("leaves a pair of rules holding prose as a pair of rules", () => {
    // `---` is a horizontal rule as well as a fence, and a note may hold as
    // many of those as it likes. What decides it is whether the contents are
    // keys, not whether two rules happen to face each other.
    const text = "# T\n\n---\n\nA break.\n\n---\n\nMore.\n";
    const split = splitNote(text);
    expect(split.frontmatter).toBeNull();
    expect(split.body).toContain("A break.");
  });

  it("does not read a rule under a line of text as a fence", () => {
    // That is a setext heading's underline, not the top of a block.
    expect(splitNote("Heading\n---\n\nBody.\n").frontmatter).toBeNull();
  });

  it("leaves a second block in the body, because a note has one set of properties", () => {
    const text = "# T\n\n---\ntags: [a]\n---\n\nBody.\n\n---\nother: b\n---\n";
    const split = splitNote(text);
    expect(split.frontmatter).toBe("tags: [a]");
    expect(split.body).toContain("other: b");
  });

  it("reads a JSON-style block, which is YAML the library already parses", () => {
    const split = splitNote('# T\n\n---\n{"tags": ["a"], "type": "note"}\n---\n');
    expect(readProperties(parseFrontmatter(split.frontmatter))).toEqual([
      { key: "tags", values: ["a"], scalar: false },
      { key: "type", values: ["note"], scalar: true },
    ]);
  });

  it("ignores a byte order mark rather than failing to find the block", () => {
    expect(splitNote(`\ufeff${NOTE}`).frontmatter).toContain("links:");
  });
});

describe("editing a note's parts", () => {
  it("puts the block back exactly where it was", () => {
    const split = splitNote(TITLED);
    expect(withFrontmatter(TITLED, split.frontmatter)).toBe(TITLED);
  });

  it("puts a first block under the title, not above it", () => {
    expect(withFrontmatter("# Just a note\n\nBody.\n", "tags: [inbox]")).toBe(
      "# Just a note\n\n---\ntags: [inbox]\n---\n\nBody.\n",
    );
  });

  it("drops the fences, and the gap under them, when the last property is removed", () => {
    expect(withFrontmatter(TITLED, "")).toBe("# Create Account Pipeline\n\nBody text.\n");
    expect(withFrontmatter(TITLED, null)).toBe("# Create Account Pipeline\n\nBody text.\n");
  });

  it("round-trips the title through the editor's view of the note", () => {
    // The editor is handed the note without its heading and hands it back the
    // same way. If this were lossy, every keystroke would rewrite the file.
    const body = withoutTitle(TITLED);
    expect(body.startsWith("---")).toBe(true);
    expect(joinTitle("Create Account Pipeline", body)).toBe(TITLED);
  });

  it("does not read a heading typed as the body's first line as the title", () => {
    // Otherwise the line being typed is lifted out from under the caret.
    const joined = joinTitle("Name", "# Section\n\ntext\n");
    expect(splitNote(joined).title).toBe("Name");
    expect(withoutTitle(joined)).toBe("# Section\n\ntext\n");
  });

  it("moves a heading written under the properties back to the top", () => {
    expect(withTitle("---\ntags: [a]\n---\n\n# Late\n\nBody.\n", "Late")).toBe(
      "# Late\n\n---\ntags: [a]\n---\n\nBody.\n",
    );
  });

  it("gives a note written without a heading one", () => {
    expect(withTitle("Just prose.\n", "untitled")).toBe("# untitled\n\nJust prose.\n");
  });
});

describe("reading properties", () => {
  it("reports a list-valued property with each of its values", () => {
    const document = parseFrontmatter(splitNote(NOTE).frontmatter);
    expect(readProperties(document)).toEqual([
      { key: "links", values: ["[[profile]]", "[[articles]]"], scalar: false },
    ]);
  });

  it("reports a scalar property as one value", () => {
    const document = parseFrontmatter("type: conversation\ncreated_at: 2026-08-16T14:30:00Z");
    expect(readProperties(document)).toEqual([
      { key: "type", values: ["conversation"], scalar: true },
      { key: "created_at", values: ["2026-08-16T14:30:00Z"], scalar: true },
    ]);
  });

  it("reports nothing for an absent block instead of failing", () => {
    expect(readProperties(parseFrontmatter(null))).toEqual([]);
  });
});

describe("editing properties", () => {
  it("leaves quoting, order, and comments alone when one value is removed", () => {
    const source = `# who this links to
links:
  - "[[profile]]"
  - "[[articles]]"
tags: [inbox]
`;
    const document = parseFrontmatter(source);
    removeValue(document, "links", "[[articles]]");

    const result = serializeFrontmatter(document);
    // The comment survives, the surviving link keeps its quotes, and `tags`
    // stays a flow sequence rather than being expanded into a block list —
    // none of which a parse-to-object round trip preserves. The library
    // normalises the spacing inside the brackets, which loses nothing.
    expect(result).toContain("# who this links to");
    expect(result).toContain('- "[[profile]]"');
    expect(result).toMatch(/tags: \[\s*inbox\s*\]/);
    expect(result).not.toContain("articles");
  });

  it("removes the key entirely once its last value goes", () => {
    const document = parseFrontmatter('links:\n  - "[[profile]]"\n');
    removeValue(document, "links", "[[profile]]");
    expect(serializeFrontmatter(document)).not.toContain("links");
  });

  it("adds a known list key as a list and anything else as a scalar", () => {
    const document = parseFrontmatter("");
    addProperty(document, "links");
    addProperty(document, "status");

    const properties = readProperties(document);
    expect(properties.find((p) => p.key === "links")?.scalar).toBe(false);
    expect(properties.find((p) => p.key === "status")?.scalar).toBe(true);
  });

  it("does not overwrite a property that is already there", () => {
    const document = parseFrontmatter('links:\n  - "[[profile]]"\n');
    addProperty(document, "links");
    expect(readProperties(document)[0]!.values).toEqual(["[[profile]]"]);
  });

  it("removes a whole property", () => {
    const document = parseFrontmatter("type: conversation\nstatus: draft\n");
    removeProperty(document, "status");
    expect(serializeFrontmatter(document)).toBe("type: conversation\n");
  });
});

describe("notes with Windows line endings", () => {
  const CRLF =
    '---\r\ntitle: Hello\r\ntags:\r\n  - a\r\n---\r\n\r\n# Body\r\n\r\nSome text\r\n';

  it("keeps carriage returns out of the values", () => {
    // The last line of the block loses its newline in the split, so a `\r` left
    // on it stops being a line break and becomes part of the value — YAML then
    // reads `a\r`, and the next property edit writes that back escaped.
    const properties = readProperties(parseFrontmatter(splitNote(CRLF).frontmatter));

    expect(properties.find((p) => p.key === "title")?.values).toEqual(["Hello"]);
    expect(properties.find((p) => p.key === "tags")?.values).toEqual(["a"]);
  });

  it("does not bake an escaped carriage return into the file on the next edit", () => {
    const document = parseFrontmatter(splitNote(CRLF).frontmatter);
    addProperty(document, "status", "draft");

    const written = serializeFrontmatter(document);
    expect(written).not.toContain("\\r");
    expect(written).toContain("status: draft");
  });

  it("leaves the body's line endings alone", () => {
    // Only the frontmatter is normalised; the note's own text is the user's.
    expect(splitNote(CRLF).body).toBe("\r\n\r\nSome text\r\n");
  });
});

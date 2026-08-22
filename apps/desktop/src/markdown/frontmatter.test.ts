/**
 * Frontmatter round-tripping (SPEC §17).
 *
 * The Properties panel edits one key of a block that agents and Obsidian also
 * write. Anything it does not touch has to come back byte for byte, or every
 * property edit becomes a diff across the whole block.
 */

import { describe, expect, it } from "vitest";

import {
  addProperty,
  joinNote,
  parseFrontmatter,
  readProperties,
  removeProperty,
  removeValue,
  serializeFrontmatter,
  splitNote,
} from "./frontmatter";

const NOTE = `---
links:
  - "[[profile]]"
  - "[[articles]]"
---

### Create Account Pipeline
`;

describe("splitting a note", () => {
  it("separates the block from the body", () => {
    const split = splitNote(NOTE);
    expect(split.frontmatter).toBe('links:\n  - "[[profile]]"\n  - "[[articles]]"');
    expect(split.body).toBe("\n### Create Account Pipeline\n");
  });

  it("rejoins to exactly what it started with", () => {
    const split = splitNote(NOTE);
    expect(joinNote(split.frontmatter, split.body)).toBe(NOTE);
  });

  it("treats a note with no block as all body", () => {
    const plain = "# Just a note\n";
    expect(splitNote(plain)).toEqual({ frontmatter: null, body: plain });
  });

  it("does not read an unterminated opening rule as frontmatter", () => {
    // The same rule heimdall-core applies on the way in: a note whose body opens
    // with a horizontal rule is a note, not a broken YAML block.
    const text = "---\nnot really yaml\n\nstill the body\n";
    expect(splitNote(text).frontmatter).toBeNull();
  });

  it("drops the fences when the last property is removed", () => {
    expect(joinNote("", "# Body\n")).toBe("# Body\n");
    expect(joinNote(null, "# Body\n")).toBe("# Body\n");
  });

  it("ignores a byte order mark rather than failing to find the block", () => {
    expect(splitNote(`﻿${NOTE}`).frontmatter).toContain("links:");
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
  const CRLF = '---\r\ntitle: Hello\r\ntags:\r\n  - a\r\n---\r\n\r\n# Body\r\n';

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
    expect(splitNote(CRLF).body).toBe("\r\n# Body\r\n");
  });
});

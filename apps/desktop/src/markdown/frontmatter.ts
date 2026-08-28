/**
 * Splitting a note into its frontmatter and its body, and putting it back.
 *
 * Edits go through `yaml`'s `Document` rather than a plain object, because the
 * Properties panel changes one key of a block that an agent — or another editor —
 * wrote. Round-tripping through a JS object would reformat every other key on
 * the first edit, which in a vault three things write to is a change signature
 * nobody asked for.
 */

import { Document, parseDocument, YAMLSeq } from "yaml";

export interface SplitNote {
  /** The raw frontmatter text, without the `---` fences. Null when absent. */
  frontmatter: string | null;
  body: string;
}

/**
 * Split a note.
 *
 * An unterminated leading `---` is not frontmatter: a note whose body opens
 * with a horizontal rule must not have the rest of itself treated as YAML.
 * This matches the rule `heimdall-core` applies on the way in.
 */
export function splitNote(text: string): SplitNote {
  const source = text.startsWith("﻿") ? text.slice(1) : text;
  const lines = source.split("\n");
  const bare = (line: string | undefined) => (line ?? "").replace(/\r$/, "");

  if (bare(lines[0]) !== "---") return { frontmatter: null, body: source };

  for (let index = 1; index < lines.length; index += 1) {
    if (bare(lines[index]) !== "---") continue;
    return {
      // Carriage returns are stripped from the block itself, not only from the
      // fences. The last line loses its newline in the join, so a `\r` left on
      // it stops being a line break and becomes part of the value: YAML then
      // reads `a\r`, and the next property edit writes that back into the file
      // as a literal escaped CR.
      frontmatter: lines.slice(1, index).map(bare).join("\n"),
      body: lines.slice(index + 1).join("\n"),
    };
  }
  return { frontmatter: null, body: source };
}

/** Reassemble a note, dropping the block entirely when it has no keys left. */
export function joinNote(frontmatter: string | null, body: string): string {
  if (frontmatter === null || frontmatter.trim() === "") return body;
  return `---\n${frontmatter.replace(/\n$/, "")}\n---\n${body}`;
}

export function parseFrontmatter(frontmatter: string | null): Document {
  // An absent block still parses, to an empty map, so callers adding the first
  // property do not need a separate path.
  return parseDocument(frontmatter ?? "");
}

export function serializeFrontmatter(document: Document): string {
  const text = String(document);
  return text.trim() === "{}" ? "" : text;
}

/** One row of the Properties table. */
export interface Property {
  key: string;
  /** A list-valued property, e.g. `links:` or `tags:`. */
  values: string[];
  /** True when the value is a single scalar rather than a list. */
  scalar: boolean;
}

/**
 * Read the block as rows for the Properties table.
 *
 * Deliberately tolerant about node shapes. A parsed document holds `Scalar`
 * nodes, but `Document.set` stores whatever it is handed — a bare string key
 * and a bare string value — so a property added in this session and one loaded
 * from disk do not look the same. Insisting on one shape would make a freshly
 * added property invisible until the note was saved and reopened.
 */
export function readProperties(document: Document): Property[] {
  const contents = document.contents;
  if (!contents || typeof contents !== "object" || !("items" in contents)) return [];

  const items = (contents as { items: unknown[] }).items;
  return items.flatMap<Property>((item) => {
    const pair = item as { key?: unknown; value?: unknown };
    const key = plain(pair.key);
    if (typeof key !== "string" || key === "") return [];

    if (pair.value instanceof YAMLSeq) {
      return [{ key, values: pair.value.items.map((entry) => String(plain(entry) ?? "")), scalar: false }];
    }
    const value = plain(pair.value);
    return [
      { key, values: value === null || value === undefined ? [] : [String(value)], scalar: true },
    ];
  });
}

/** Unwrap a YAML node to its value, passing a bare value straight through. */
function plain(node: unknown): unknown {
  if (node !== null && typeof node === "object" && "value" in node) {
    return (node as { value: unknown }).value;
  }
  return node;
}

/**
 * Keys that hold a list of things rather than one thing.
 *
 * The vault template's own `types.json` registers `links` as `multitext`, and
 * `tags` and `aliases` are the conventional built-ins. Anything else defaults to a
 * scalar, which is what a new property usually wants to be.
 */
const LIST_KEYS = ["links", "tags", "aliases", "cssclasses"];

export function isListKey(key: string): boolean {
  return LIST_KEYS.includes(key.toLowerCase());
}

/**
 * Add a property, or add a value to one that already exists.
 *
 * A list key with a value appends to the list rather than replacing it, which is
 * what "add another link" means; a scalar takes the value as written.
 */
export function addProperty(document: Document, key: string, value = ""): void {
  if (isListKey(key)) {
    const existing = document.get(key, true);
    const list = existing instanceof YAMLSeq ? existing : new YAMLSeq();
    if (!(existing instanceof YAMLSeq)) document.set(key, list);
    if (value !== "") list.add(value);
    return;
  }

  if (document.has(key) && value === "") return;
  document.set(key, value);
}

export function removeProperty(document: Document, key: string): void {
  document.delete(key);
}

/** Drop one value from a list-valued property, and the key if it empties. */
export function removeValue(document: Document, key: string, value: string): void {
  const node = document.get(key, true);
  if (!(node instanceof YAMLSeq)) {
    document.delete(key);
    return;
  }
  node.items = node.items.filter((item) => String(plain(item) ?? "") !== value);
  if (node.items.length === 0) document.delete(key);
}

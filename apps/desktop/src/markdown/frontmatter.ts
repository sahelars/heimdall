/**
 * A note's three parts — its title, its frontmatter block, and its body — and
 * the edits that replace either of the first two without disturbing the rest.
 *
 * Edits go through `yaml`'s `Document` rather than a plain object, because the
 * Properties panel changes one key of a block that an agent — or another editor —
 * wrote. Round-tripping through a JS object would reformat every other key on
 * the first edit, which in a vault three things write to is a change signature
 * nobody asked for.
 *
 * The same reasoning applies a level up, which is why the block is replaced
 * where it stands rather than lifted out and reassembled: a note is text
 * somebody laid out, and a property edit is not licence to move it.
 */

import { Document, parseDocument, YAMLSeq } from "yaml";

const BOM = "﻿";

/** The leading `# ` heading, with the marker and the space that follows it. */
const TITLE = /^#[ \t]+(.+?)[ \t]*$/;

export interface SplitNote {
  /** The title heading's text, marker stripped. Null when the note has none. */
  title: string | null;
  /** The raw frontmatter text, without the `---` fences. Null when absent. */
  frontmatter: string | null;
  /** Everything else, in order, with the title line and the block taken out. */
  body: string;
}

/** A frontmatter block, as the line indices of its two fences. */
export interface Block {
  start: number;
  end: number;
}

/**
 * A line with its carriage return taken off.
 *
 * Stripped from the block's own lines, not only from its fences. The last line
 * loses its newline when the block is joined, so a `\r` left on it stops being a
 * line break and becomes part of the value: YAML then reads `a\r`, and the next
 * property edit writes that back into the file as a literal escaped CR.
 */
function bare(line: string | undefined): string {
  return (line ?? "").replace(/\r$/, "");
}

function isFence(line: string | undefined): boolean {
  return bare(line).trimEnd() === "---";
}

function isBlank(line: string | undefined): boolean {
  return bare(line).trim() === "";
}

/** Whether a block's contents are properties rather than prose. */
function isProperties(inner: string): boolean {
  if (inner.trim() === "") return false;
  const document = parseDocument(inner);
  // A mapping, and only a mapping. Prose between two rules parses cleanly as a
  // YAML scalar, so "it parsed" is not the test — "it has keys" is. Flow
  // mappings come back the same way, which is what makes a JSON-style block
  // work with no separate path.
  return document.errors.length === 0 && readProperties(document).length > 0;
}

/**
 * Find the note's frontmatter block, wherever its author put it.
 *
 * A block is not required to open the file: a note reads `# Title` first and its
 * properties second, and one written by hand may put them further down still.
 * What a block *is* required to be is unambiguous, because `---` is also a
 * horizontal rule, and a note may hold as many of those as it likes:
 *
 * - the fence is a line of its own, at the top of the note or under a blank
 *   line — a `---` directly beneath text is a setext heading's underline;
 * - it is closed;
 * - and what it encloses parses as a non-empty mapping.
 *
 * A candidate failing any of those is passed over and the scan goes on, so its
 * `---` lines reach the renderer as the rules they are. The **first** block that
 * qualifies is the note's properties and the only one: a second is left in the
 * body, because a note has one set of properties and a panel showing two of them
 * could not say which the file would keep.
 */
export function findFrontmatterBlock(lines: readonly string[]): Block | null {
  for (let start = 0; start < lines.length; start += 1) {
    if (!isFence(lines[start])) continue;
    if (start > 0 && !isBlank(lines[start - 1])) continue;

    for (let end = start + 1; end < lines.length; end += 1) {
      if (!isFence(lines[end])) continue;
      if (isProperties(lines.slice(start + 1, end).map(bare).join("\n"))) return { start, end };
      // Its closing fence belongs to something else. Give up on this opening
      // one rather than reading on to a later `---`, which would swallow
      // everything between two ordinary rules.
      break;
    }
  }
  return null;
}

/**
 * The line the note's title sits on.
 *
 * The title is the note's first line of content — not merely its first heading.
 * A `## Later section` halfway down a note is a section, and hoisting it into
 * the header because the note happens to open without a heading would be a
 * surprise nobody could undo. Blank lines and the frontmatter block are stepped
 * over, so a note that leads with its properties still has a title under them.
 */
function titleLineOf(lines: readonly string[], block: Block | null): number | null {
  for (let index = 0; index < lines.length; index += 1) {
    if (block && index >= block.start && index <= block.end) {
      index = block.end;
      continue;
    }
    if (isBlank(lines[index])) continue;
    return TITLE.test(bare(lines[index])) ? index : null;
  }
  return null;
}

function withoutBom(text: string): { bom: string; source: string } {
  return text.startsWith(BOM) ? { bom: BOM, source: text.slice(1) } : { bom: "", source: text };
}

/** Split a note into the three parts the preview renders separately. */
export function splitNote(text: string): SplitNote {
  const { source } = withoutBom(text);
  const lines = source.split("\n");
  const block = findFrontmatterBlock(lines);
  const title = titleLineOf(lines, block);

  return {
    title: title === null ? null : (TITLE.exec(bare(lines[title]!))?.[1] ?? null),
    frontmatter: block ? lines.slice(block.start + 1, block.end).map(bare).join("\n") : null,
    body: lines
      .filter((_, index) => {
        if (block && index >= block.start && index <= block.end) return false;
        return index !== title;
      })
      .join("\n"),
  };
}

/**
 * Replace the note's frontmatter block, leaving everything else alone.
 *
 * An empty block is removed rather than written out as two bare fences, which
 * would read as a rule the user never typed. A note with no block yet gets one
 * under its title, where the next editor to open the file will expect it.
 */
export function withFrontmatter(text: string, frontmatter: string | null): string {
  const { bom, source } = withoutBom(text);
  const lines = source.split("\n");
  const block = findFrontmatterBlock(lines);
  const empty = frontmatter === null || frontmatter.trim() === "";
  const fenced = empty ? [] : ["---", ...frontmatter.replace(/\n$/, "").split("\n"), "---"];

  if (block) {
    // Removing the block takes the blank line under it too; leaving it behind
    // would open the note with a gap that grows by one every time the last
    // property is deleted and added again.
    const end = empty && isBlank(lines[block.end + 1]) ? block.end + 1 : block.end;
    return bom + [...lines.slice(0, block.start), ...fenced, ...lines.slice(end + 1)].join("\n");
  }
  if (empty) return text;

  const title = titleLineOf(lines, null);
  if (title === null) return bom + [...fenced, "", ...lines].join("\n");

  const spaced = isBlank(lines[title + 1]);
  const at = spaced ? title + 2 : title + 1;
  return (
    bom +
    [...lines.slice(0, at), ...(spaced ? [] : [""]), ...fenced, "", ...lines.slice(at)].join("\n")
  );
}

/**
 * The note without its title line — what the source editor shows.
 *
 * The heading is not hidden: it is drawn above the editor, where it cannot be
 * scrolled away from or deleted. Showing it in both places would be showing the
 * same fact twice, and only one of the two could be typed into.
 *
 * The blank line under the heading goes with it, so that putting the title back
 * returns exactly the bytes that were there.
 */
export function withoutTitle(text: string): string {
  const { bom, source } = withoutBom(text);
  const lines = source.split("\n");
  const title = titleLineOf(lines, findFrontmatterBlock(lines));
  if (title === null) return text;

  const end = isBlank(lines[title + 1]) ? title + 1 : title;
  return bom + [...lines.slice(0, title), ...lines.slice(end + 1)].join("\n");
}

/**
 * Put a title on top of a body the editor returned.
 *
 * Deliberately literal: whatever the editor holds is the body, and a `# ` the
 * user has just typed as its first line is a heading in the body, not a second
 * title. Reading it as a title would lift the line they were typing out from
 * under the cursor.
 */
export function joinTitle(title: string, body: string): string {
  const { bom, source } = withoutBom(body);
  return `${bom}# ${title}\n\n${source}`;
}

/**
 * Retitle a note.
 *
 * Always on top, and always present: the heading is the note's name, so a note
 * cannot be left without one, and a name buried under a paragraph is not a name
 * anybody would find. A note whose properties were written above its heading is
 * the one case where this moves something, and it moves it once.
 */
export function withTitle(text: string, title: string): string {
  return joinTitle(title, withoutTitle(text).replace(/^(﻿)?\n+/, "$1"));
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

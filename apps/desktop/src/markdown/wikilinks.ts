/**
 * Wikilinks, as the preview and the properties table need them.
 *
 * Resolution mirrors what `link_graph` does in Rust — vault-relative first, then
 * relative to the linking note, then shortest basename match. It is duplicated
 * rather than shared because the two run at different moments: the index is
 * built once per vault, and a preview has to resolve a link the instant someone
 * types it, before any index has been rebuilt.
 */

export interface Wikilink {
  /** The note being pointed at, before resolution. */
  target: string;
  /** What the link should read as. */
  label: string;
}

/** Pull the target and label out of the inside of a `[[...]]`. */
export function parseWikilink(inner: string): Wikilink | null {
  const [before = "", alias] = inner.split("|", 2);
  const target = (before.split("#")[0] ?? "").trim();
  if (target === "") return null;

  const heading = before.includes("#") ? before.slice(before.indexOf("#") + 1).trim() : "";
  const label = alias?.trim() || (heading ? `${target} › ${heading}` : target);
  return { target, label };
}

/** Split text into plain runs and wikilinks, in order. */
export function splitWikilinks(text: string): (string | Wikilink)[] {
  const parts: (string | Wikilink)[] = [];
  let rest = text;

  while (true) {
    const start = rest.indexOf("[[");
    if (start === -1) break;
    const end = rest.indexOf("]]", start + 2);
    if (end === -1) break;

    if (start > 0) parts.push(rest.slice(0, start));
    const link = parseWikilink(rest.slice(start + 2, end));
    // `[[#heading]]` points inside the same note; there is nothing to link to.
    parts.push(link ?? rest.slice(start, end + 2));
    rest = rest.slice(end + 2);
  }

  if (rest !== "") parts.push(rest);
  return parts;
}

/**
 * Turn a written target into a vault path, or null when nothing matches.
 *
 * `paths` is every note in the vault. The tie-break is the same total order the
 * Rust index uses, so the preview and the graph never disagree about where a
 * link goes.
 */
export function resolveWikilink(target: string, from: string, paths: readonly string[]): string | null {
  const key = normalize(target);
  if (!key) return null;

  const lower = new Map(paths.map((path) => [path.toLowerCase(), path]));

  const exact = lower.get(key.toLowerCase());
  if (exact) return exact;

  const parent = from.includes("/") ? from.slice(0, from.lastIndexOf("/")) : "";
  const relative = parent ? `${parent}/${key}` : key;
  const nearby = lower.get(relative.toLowerCase());
  if (nearby) return nearby;

  const name = key.slice(key.lastIndexOf("/") + 1).toLowerCase();
  const candidates = paths.filter((path) => path.slice(path.lastIndexOf("/") + 1).toLowerCase() === name);
  if (candidates.length === 0) return null;

  const exactCase = candidates.filter((path) => path.endsWith(key));
  const pool = exactCase.length > 0 ? exactCase : candidates;

  return [...pool].sort((a, b) => {
    const depth = a.split("/").length - b.split("/").length;
    if (depth !== 0) return depth;
    if (a.length !== b.length) return a.length - b.length;
    // A total order, so two runs over one vault cannot disagree.
    return a < b ? -1 : 1;
  })[0]!;
}

function normalize(target: string): string | null {
  const cleaned = target.replace(/\\/g, "/").trim().replace(/^\.\//, "").replace(/\/$/, "");
  if (cleaned === "") return null;
  const name = cleaned.slice(cleaned.lastIndexOf("/") + 1);
  // Only `.md` files are notes, so a bare name means the Markdown file.
  return /\.md$/i.test(name) ? cleaned : `${cleaned}.md`;
}

/**
 * Which operation reads and writes a given vault path.
 *
 * Ordinary notes go through `read-documents` / `write-document`. The protected
 * tree does not: `read-documents` excludes `aios/` at every depth by design
 * (SPEC §6), so a note there has to be reached by the operation that owns it.
 * Every caller that opens or saves a path dispatches on this rather than
 * assuming one pair of commands covers the vault.
 */

export type DocumentSource =
  | "document"
  | "memory-main"
  | "memory-extended"
  | "entry-conversation"
  | "entry-notification";

const MAIN_MEMORY = "aios/memories/memory.md";
const EXTENDED_PREFIX = "aios/memories/extended/";
const CONVERSATIONS_PREFIX = "aios/conversations/";
const NOTIFICATIONS_PREFIX = "aios/notifications/";

/**
 * Compare the way the vault contract does.
 *
 * `aios/` is matched case-insensitively for the reason SPEC §6 gives: on macOS
 * and Windows `AIOS/notes.md` and `aios/notes.md` are the same file, so a
 * case-sensitive check here would route a protected path to the ordinary
 * commands, which would then report it as missing.
 */
function is(path: string, other: string): boolean {
  return path.toLowerCase() === other.toLowerCase();
}

function startsWith(path: string, prefix: string): boolean {
  return path.toLowerCase().startsWith(prefix.toLowerCase());
}

export function sourceOf(path: string): DocumentSource {
  if (is(path, MAIN_MEMORY)) return "memory-main";
  if (startsWith(path, EXTENDED_PREFIX)) return "memory-extended";
  if (startsWith(path, CONVERSATIONS_PREFIX)) return "entry-conversation";
  if (startsWith(path, NOTIFICATIONS_PREFIX)) return "entry-notification";
  return "document";
}

export function isProtected(path: string): boolean {
  return path.toLowerCase() === "aios" || startsWith(path, "aios/");
}

/** The filename, without its folders. */
export function baseName(path: string): string {
  return path.slice(path.lastIndexOf("/") + 1);
}

/** The filename without its extension — what the UI shows as a note's name. */
export function titleOf(path: string): string {
  const name = baseName(path);
  const dot = name.lastIndexOf(".");
  return dot > 0 ? name.slice(0, dot) : name;
}

/** The folder holding a path, or `""` for the vault root. */
export function parentOf(path: string): string {
  const slash = path.lastIndexOf("/");
  return slash === -1 ? "" : path.slice(0, slash);
}

/**
 * Path helpers for vault-relative paths.
 *
 * Every note in the vault is read and written the same way (`read` / `write`),
 * so there is no routing here — only the small string operations the UI keeps
 * reaching for.
 */

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

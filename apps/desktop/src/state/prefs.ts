/**
 * Preferences that survive a restart, in one namespace.
 *
 * Storage is a convenience here, never a requirement: a window that cannot
 * reach it still starts, with defaults.
 */

const PREFIX = "heimdall.";

export function readPref<T>(key: string, fallback: T, accepts: (value: unknown) => value is T): T {
  try {
    const raw = window.localStorage.getItem(PREFIX + key);
    if (raw === null) return fallback;

    let parsed: unknown;
    try {
      parsed = JSON.parse(raw);
    } catch {
      // An earlier build stored the vault path as a bare string rather than as
      // JSON. Reading it back is a one-line kindness; the alternative is
      // silently forgetting which vault someone was working in.
      parsed = raw;
    }
    return accepts(parsed) ? parsed : fallback;
  } catch {
    return fallback;
  }
}

export function writePref(key: string, value: unknown): void {
  try {
    window.localStorage.setItem(PREFIX + key, JSON.stringify(value));
  } catch {
    // Remembering a pane width is not worth failing a save over.
  }
}

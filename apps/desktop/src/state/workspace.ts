/**
 * Workspace state, as pure functions.
 *
 * History, dirty tracking, and conflict handling all live here rather than in a
 * component, because they are the parts that have to be right and the parts a
 * jsdom render cannot exercise properly.
 */

export interface HistoryState {
  entries: string[];
  index: number;
}

export const EMPTY_HISTORY: HistoryState = { entries: [], index: -1 };

/** How many notes back the arrows can reach. */
const HISTORY_LIMIT = 50;

export function canGoBack(history: HistoryState): boolean {
  return history.index > 0;
}

export function canGoForward(history: HistoryState): boolean {
  return history.index < history.entries.length - 1;
}

export function currentPath(history: HistoryState): string | null {
  return history.entries[history.index] ?? null;
}

/**
 * Open a note.
 *
 * Re-opening the note already showing is not a new entry — otherwise clicking
 * the same tree row twice would fill the history with itself. Opening anything
 * else truncates the forward branch, the way a browser does.
 */
export function visit(history: HistoryState, path: string): HistoryState {
  if (currentPath(history) === path) return history;

  const kept = history.entries.slice(0, history.index + 1);
  const entries = [...kept, path].slice(-HISTORY_LIMIT);
  return { entries, index: entries.length - 1 };
}

export function goBack(history: HistoryState): HistoryState {
  return canGoBack(history) ? { ...history, index: history.index - 1 } : history;
}

export function goForward(history: HistoryState): HistoryState {
  return canGoForward(history) ? { ...history, index: history.index + 1 } : history;
}

/**
 * Forget a note that no longer exists.
 *
 * After a delete or a rename, leaving the old path in the history would give
 * the back arrow a destination that fails to open.
 */
export function forget(history: HistoryState, path: string): HistoryState {
  const entries = history.entries.filter((entry) => entry !== path);
  if (entries.length === history.entries.length) return history;

  const removedBefore = history.entries
    .slice(0, history.index + 1)
    .filter((entry) => entry === path).length;
  return { entries, index: Math.min(history.index - removedBefore, entries.length - 1) };
}

/** A note held open in the editor. */
export interface Buffer {
  path: string;
  /** Exactly what the last complete read returned. */
  disk: string;
  /** What the editor holds now. */
  buffer: string;
  /** The revision that read reported, or null for a note not yet on disk. */
  revision: string | null;
  editable: boolean;
  saving: boolean;
  conflict: { theirRevision: string } | null;
}

/**
 * Whether there is anything to save.
 *
 * Derived rather than tracked, so it cannot drift out of step with the text the
 * way a boolean flag set in an event handler can.
 */
export function isDirty(buffer: Buffer): boolean {
  return buffer.buffer !== buffer.disk;
}

/**
 * Whether an autosave should run.
 *
 * A conflict suspends it. Otherwise every autosave would fail on the same stale
 * revision, and the note would sit in a loop the user could not read past.
 */
export function shouldAutosave(buffer: Buffer): boolean {
  return buffer.editable && isDirty(buffer) && !buffer.saving && buffer.conflict === null;
}

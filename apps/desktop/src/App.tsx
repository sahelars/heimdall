/**
 * The application: one vault, three panes, and Settings behind the menu.
 *
 * Heimdall Desktop is where the user works on their vault — a file tree, a
 * Markdown editor with preview, and the link graph (SPEC §15). Setup and
 * diagnostics moved into a modal, because they are occasional and the window
 * belongs to the notes.
 */

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";

import { cliStatus, listVaults, vaultName } from "./api/cli";
import {
  CliFailure,
  createFolder,
  deletePath,
  listAllDocuments,
  lockPath,
  movePath,
  readWholeDocument,
  readLockState,
  relinkPaths,
  saveDocument,
  unlockPath,
} from "./api/documents";
import { backlinksOf, backlinksUnder, loadVaultIndex, type VaultIndex } from "./api/index-graph";
import { baseName, parentOf, titleOf } from "./api/source";
import type { CliStatus, DomainError, LinkGraphData, RelinkData } from "./api/types";
import { Button, Failure } from "./components";
import { withTitle } from "./markdown/frontmatter";
import { Explorer } from "./features/explorer/Explorer";
import { containsLocked, type TreeInput, type TreeNode } from "./features/explorer/tree";
import { GraphPane } from "./features/graph/GraphPane";
import { Mermaid } from "./features/note/Mermaid";
import { NoteView, type OpenNote, type ViewMode } from "./features/note/NoteView";
import type { RecordedFailure } from "./features/settings/Diagnostics";
import { SettingsModal } from "./features/settings/SettingsModal";
import { QuickSwitcher } from "./features/switcher/QuickSwitcher";
import { Prompt } from "./components/Prompt";
import { ContextMenu, type MenuItem } from "./components/ContextMenu";
import { Confirm } from "./components/Confirm";
import { Modal } from "./components/Modal";
import { Workspace } from "./features/workspace/Workspace";
import { defaultPanes, isPaneWidths, type PaneWidths } from "./features/workspace/panes";
import { readPref, writePref } from "./state/prefs";
import { applyAccents, isAccentsPreference, NO_ACCENTS, toAccents } from "./state/accent";
import {
  applyTheme,
  isThemePreference,
  resolveTheme,
  systemPrefersDark,
  watchSystemTheme,
  type ThemePreference,
} from "./state/theme";
import {
  canGoBack,
  canGoForward,
  currentPath,
  EMPTY_HISTORY,
  forget,
  goBack,
  goForward,
  visit,
  type HistoryState,
} from "./state/workspace";
import { resolveWikilink } from "./markdown/wikilinks";

/**
 * Where a new note or folder goes: beside whatever is open.
 *
 * A locked folder is not skipped here. The CLI refuses a create inside one with
 * `LOCKED`, and saying so in a dialog is better than quietly putting the note
 * somewhere the user did not choose.
 */
function destinationFolder(openPath: string | null): string {
  return openPath ? parentOf(openPath) : "";
}

/**
 * What a `LOCKED` refusal says, in words.
 *
 * `locked_at` names the folder or note whose rule applies, and `""` is the vault
 * root — which is the part worth telling someone, since the rule that stopped
 * them is often on a folder rather than on what they touched.
 */
export function describeLocked(error: DomainError): string {
  const path = typeof error.details?.path === "string" ? error.details.path : null;
  const at = typeof error.details?.locked_at === "string" ? error.details.locked_at : null;
  const what = path ? `"${path}"` : "That path";
  if (at === null) return `${what} is locked, so it cannot be changed.`;
  const where = at === "" ? "the whole vault is locked" : at === path ? "it is locked" : `"${at}" is locked`;
  return `${what} cannot be changed: ${where}. Unlock it to make this change.`;
}

/** `n thing` or `n things`, which is most of what a report has to say. */
function plural(count: number, noun: string): string {
  return `${count} ${noun}${count === 1 ? "" : "s"}`;
}

/**
 * What a move failed to carry, or null when it carried everything.
 *
 * Only shortfalls are reported. The user agreed to the rewrite before it ran
 * and can see the result in front of them, so telling them it worked is a
 * dialog to dismiss for no information. A link left pointing at a name that is
 * gone is the opposite: invisible until they follow it, and the one thing they
 * cannot be left to discover on their own.
 */
export function describeShortfall(result: RelinkData): string | null {
  const said: string[] = [];

  if (result.skipped.length > 0) {
    const where = [...new Set(result.skipped.map((skip) => skip.path))];
    said.push(
      `${plural(result.skipped.length, "link")} could not be rewritten and ${
        result.skipped.length === 1 ? "was" : "were"
      } left pointing at the old name, in ${where.slice(0, 3).join(", ")}${
        where.length > 3 ? ` and ${where.length - 3} more` : ""
      }.`,
    );
  }
  const locked = result.locked ?? [];
  if (locked.length > 0) {
    said.push(
      `${plural(locked.length, "locked note")} ${
        locked.length === 1 ? "links" : "link"
      } here and ${locked.length === 1 ? "was" : "were"} left unchanged because ${
        locked.length === 1 ? "it is" : "they are"
      } locked: ${locked.slice(0, 3).join(", ")}${
        locked.length > 3 ? ` and ${locked.length - 3} more` : ""
      }.`,
    );
  }
  if (result.truncated.file_cap_hit || result.truncated.total_bytes_cap_hit) {
    said.push(
      `${plural(result.truncated.files_unscanned, "note")} were too large or too many to check.`,
    );
  }

  return said.length > 0 ? said.join(" ") : null;
}

/** What a move did to the rest of the vault, once its follow-up has run. */
interface MoveOutcome {
  /** The notes whose links were rewritten; one of them may be on screen. */
  rewritten: string[];
  /** What it could not carry, or null when it carried everything. */
  shortfall: string | null;
  /** Set when the rewrite failed outright. The move itself still landed. */
  error?: DomainError;
}

/** One path change, however it was asked for. */
interface MoveRequest {
  from: string;
  to: string;
  kind: "document" | "directory";
  /**
   * Open the note afterwards.
   *
   * A rename reveals its result — you named it, so you are looking at it. A
   * drag does not: dropping a note into a folder is filing, and having the
   * pane jump to whatever was filed is not what was asked for.
   */
  reveal: boolean;
  /** A name typed into the note's heading, to write before the move. */
  heading?: string;
}

/**
 * A move waiting to be agreed to.
 *
 * Every move that would break inbound links stops here first. The dialog is the
 * only thing that starts one, so a rewrite of notes the user never opened is
 * always something they asked for by name.
 */
interface PendingMove extends MoveRequest {
  /** The notes that link in, which the dialog lists. */
  linking: string[];
  /** Those of them that are locked, which `relink` will leave unchanged. */
  lockedLinking: string[];
}

const SETTINGS_EVENT = "menu:settings";

/** How long after the last keystroke an autosave runs. */
const AUTOSAVE_DELAY = 1500;

/**
 * How long after a write the index is rebuilt.
 *
 * `link-graph` reads every note in the vault, so it is a load-and-refresh
 * operation rather than an interactive one (SPEC §15). Just long enough to
 * coalesce a burst of writes — autosave is already debounced, so this only has
 * to catch a save landing next to a rename, and any longer leaves the graph and
 * the mentions panel visibly behind the note.
 */
const INDEX_REFRESH_DELAY = 250;

export function App() {
  // Read in the initialiser rather than in an effect: doing it after the first
  // paint flashes "No vault yet" on every launch before the workspace appears.
  const [vault, setVault] = useState(() =>
    readPref("vault", "", (value): value is string => typeof value === "string"),
  );
  const [status, setStatus] = useState<CliStatus | null>(null);
  // The folder name of a vault forgotten because its folder was deleted, for
  // the empty screen to say what happened to it.
  const [forgotten, setForgotten] = useState<string | null>(null);
  const [failures, setFailures] = useState<RecordedFailure[]>([]);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [switcherOpen, setSwitcherOpen] = useState(false);
  const [prompt, setPrompt] = useState<"note" | "folder" | null>(null);
  const [menu, setMenu] = useState<{ node: TreeNode; x: number; y: number } | null>(null);
  const [renaming, setRenaming] = useState<TreeNode | null>(null);
  const [deleting, setDeleting] = useState<TreeNode | null>(null);
  const [actionError, setActionError] = useState<DomainError | null>(null);
  /**
   * A write refused because something is locked.
   *
   * A dialog, never the banner: autosave triggers a reload 1.5 seconds after
   * any edit, and a successful reload clears the banner before it is read.
   */
  const [lockedError, setLockedError] = useState<DomainError | null>(null);
  /**
   * Whether the banner is showing a failure `reload` itself put there.
   *
   * `reload` clears the banner when the vault comes back, which is right for
   * its own "this folder is not a vault" and wrong for everything else: a
   * background refresh — autosave triggers one 1.5 seconds after any edit —
   * would wipe the report of a write the user had just made, before they had
   * read it.
   */
  const bannerFromReload = useRef(false);

  /** A move that will break inbound links, waiting to be agreed to. */
  const [pendingMove, setPendingMove] = useState<PendingMove | null>(null);
  /** What the last move could not carry. Only shortfalls are reported. */
  const [shortfall, setShortfall] = useState<string | null>(null);

  const [theme, setTheme] = useState<ThemePreference>(() =>
    readPref("theme", "system" as ThemePreference, isThemePreference),
  );
  // One accent per theme; null in either is the stylesheet's own, which is
  // where that theme's default lives. Normalized on the way in, so a value
  // stored before the accent was split still reads back as dark mode's.
  const [accents, setAccents] = useState(() =>
    toAccents(readPref("accent", NO_ACCENTS, isAccentsPreference)),
  );
  // Resolved against the real window, which the workspace spans: the window
  // opens maximized, so a pixel default written for one screen is wrong on the
  // next. A stored preference still wins — someone who has dragged their panes
  // keeps them.
  const [panes, setPanes] = useState<PaneWidths>(() =>
    readPref("panes", defaultPanes(window.innerWidth), isPaneWidths),
  );
  const [prefersDark, setPrefersDark] = useState(systemPrefersDark);

  const [index, setIndex] = useState<VaultIndex | null>(null);
  const [listing, setListing] = useState<TreeInput[]>([]);
  const [rootLocked, setRootLocked] = useState(false);
  const [truncated, setTruncated] = useState(false);

  const [history, setHistory] = useState<HistoryState>(EMPTY_HISTORY);
  const [note, setNote] = useState<OpenNote | null>(null);
  const [mode, setMode] = useState<ViewMode>("preview");
  const [loading, setLoading] = useState(false);

  /** The last complete read, so a save knows what revision it is replacing. */
  const disk = useRef<{ path: string; text: string; revision: string | null } | null>(null);
  const refreshTimer = useRef<number | undefined>(undefined);
  /**
   * Whether a write is already in flight.
   *
   * A ref rather than the `saving` flag on the note: that flag is React state
   * and does not update until the next render, so two saves fired in the same
   * tick — CodeMirror's own ⌘S keymap and the window handler both firing — would
   * both read the same revision, and the second would come back as a conflict
   * the user never caused.
   */
  /** Serialises writes, so a flush can await whatever is already in the air. */
  const queue = useRef<Promise<void>>(Promise.resolve());
  /** The exact bytes whose write failed, so autosave does not retry them forever. */
  const failedAt = useRef<{ path: string; text: string } | null>(null);
  /** The open note as it stands now, for handlers that would otherwise be stale. */
  const noteNow = useRef<OpenNote | null>(null);
  /**
   * Which open request is the current one.
   *
   * Reads are asynchronous and a large note takes several round trips, so a
   * click on a slow note followed by a click on a fast one would otherwise let
   * the slow read land last and replace the note the user is actually looking
   * at.
   */
  const opening = useRef(0);
  /**
   * The history as it stands right now.
   *
   * The state value a handler closes over is the one from its render, so two
   * quick Back clicks would both compute a step from the same starting point
   * and only move once.
   */
  const historyNow = useRef<HistoryState>(EMPTY_HISTORY);
  /** Writes anything unsaved. Held in a ref so `open` can call it before `save` exists. */
  const flush = useRef<() => Promise<boolean>>(async () => true);
  /** Held in a ref so `step` can reach `open` without depending on it. */
  const openRef = useRef<(path: string) => Promise<void>>(async () => {});
  const openPath = currentPath(history);

  historyNow.current = history;
  noteNow.current = note;

  /** Step through history, from wherever it actually is. */
  const step = useCallback(
    (move: (state: HistoryState) => HistoryState) => {
      const next = move(historyNow.current);
      if (next === historyNow.current) return;

      historyNow.current = next;
      setHistory(next);
      const path = currentPath(next);
      if (path) void openRef.current(path);
    },
    [],
  );

  const record = useCallback((context: string, error: DomainError) => {
    setFailures((previous) => [{ at: new Date().toISOString(), context, error }, ...previous]);
  }, []);

  /* Theme and preferences ------------------------------------------------- */

  useEffect(() => {
    applyTheme(document.documentElement, theme);
    writePref("theme", theme);
  }, [theme]);

  useEffect(() => {
    applyAccents(document.documentElement, accents);
    writePref("accent", accents);
  }, [accents]);

  useEffect(() => writePref("panes", panes), [panes]);

  // The stylesheet follows the platform on its own, but a mermaid diagram is
  // drawn rather than styled and has to be told to redraw.
  useEffect(() => watchSystemTheme(setPrefersDark), []);

  /* The bridge ------------------------------------------------------------ */

  const refreshStatus = useCallback(() => {
    cliStatus()
      .then((next) => {
        setStatus(next);
        if (next.error) record("cli_status", next.error);
      })
      .catch(() => setStatus(null));
  }, [record]);

  useEffect(refreshStatus, [refreshStatus]);

  /* Heimdall → Settings… --------------------------------------------------- */

  useEffect(() => {
    let dispose: (() => void) | undefined;
    let cancelled = false;

    // `listen` resolves asynchronously and StrictMode mounts twice, so the
    // handle can arrive after unmount. Both cases are handled.
    listen(SETTINGS_EVENT, () => setSettingsOpen(true))
      .then((unlisten) => {
        if (cancelled) unlisten();
        else dispose = unlisten;
      })
      .catch(() => {
        // A browser with no Tauri bridge still renders the workspace.
      });

    return () => {
      cancelled = true;
      dispose?.();
    };
  }, []);

  /* The vault ------------------------------------------------------------- */

  /** Put a failure on the banner and keep it there until it is dismissed. */
  const showError = useCallback((error: DomainError) => {
    // A lock is not a failure to dismiss and forget: it names what to unlock,
    // and it has to survive the reload that follows every write.
    if (error.code === "LOCKED") {
      setLockedError(error);
      return;
    }
    bannerFromReload.current = false;
    setActionError(error);
  }, []);

  const reload = useCallback(
    async (target: string) => {
      if (!target) return;
      try {
        const [nextIndex, nextListing] = await Promise.all([
          loadVaultIndex(target),
          listAllDocuments(target),
        ]);
        setIndex(nextIndex);
        setTruncated(nextListing.truncated);
        setRootLocked(nextListing.rootLocked);
        // The tree is the listing alone: it supplies every folder, including
        // empty ones, and the lock state of each path.
        setListing(
          nextListing.entries.map((entry) => ({
            path: entry.path,
            kind: entry.kind,
            locked: entry.locked,
          })),
        );
        if (bannerFromReload.current) {
          bannerFromReload.current = false;
          setActionError(null);
        }
      } catch (thrown) {
        if (thrown instanceof CliFailure && thrown.error.details?.reason === "vault_missing") {
          // The vault's folder is not there. Listing the vaults forgets one
          // that was deleted; if it is gone from the list, it is gone, and
          // the workspace says so and goes back to "No vault yet" rather
          // than raising the same banner on every launch. One that is still
          // listed — a drive that is unplugged — keeps the banner below.
          try {
            const listed = await listVaults();
            const still = listed.ok
              ? listed.data?.vaults.some(
                  (known) => known.path === target || known.path === `/private${target}`,
                )
              : true;
            if (!still) {
              bannerFromReload.current = false;
              setActionError(null);
              setForgotten(vaultName(target));
              setVault("");
              writePref("vault", "");
              setHistory(EMPTY_HISTORY);
              setNote(null);
              setIndex(null);
              setListing([]);
              setRootLocked(false);
              return;
            }
          } catch {
            // Fall through to the banner: not knowing is not "deleted".
          }
        }
        if (thrown instanceof CliFailure) {
          record("link-graph", thrown.error);
          // Its own, so its own success may clear it again.
          bannerFromReload.current = true;
          // A vault that has moved, or cannot be read, otherwise leaves
          // the workspace showing an empty tree and "Building the graph…"
          // forever with the reason buried in Diagnostics.
          setActionError(thrown.error);
          setIndex(null);
          setListing([]);
          setRootLocked(false);
        }
      }
    },
    [record],
  );

  useEffect(() => {
    void reload(vault);
  }, [vault, reload]);

  /** Rebuild the index once a burst of writes has settled. */
  const scheduleReload = useCallback(
    (target: string) => {
      window.clearTimeout(refreshTimer.current);
      refreshTimer.current = window.setTimeout(() => void reload(target), INDEX_REFRESH_DELAY);
    },
    [reload],
  );

  useEffect(() => () => window.clearTimeout(refreshTimer.current), []);

  /**
   * Catch up on edits made elsewhere.
   *
   * Nothing watches the filesystem, so a note changed in another editor or by an
   * agent while this window was in the background would otherwise show stale
   * links until something else happened to trigger a reload.
   */
  useEffect(() => {
    if (!vault) return;
    const onFocus = () => scheduleReload(vault);
    window.addEventListener("focus", onFocus);
    return () => window.removeEventListener("focus", onFocus);
  }, [vault, scheduleReload]);

  const chooseVault = useCallback((next: string) => {
    setForgotten(null);
    setVault(next);
    writePref("vault", next);
    setHistory(EMPTY_HISTORY);
    setNote(null);
  }, []);

  /* Opening notes --------------------------------------------------------- */

  const paths = useMemo(() => index?.nodes.map((node) => node.path) ?? [], [index]);

  const open = useCallback(
    async (path: string) => {
      // Autosave is debounced, and switching notes cancels its pending timer —
      // so without this, edits made in the last second and a half before a click
      // would be dropped on the floor with nothing said.
      //
      // And if that write does not land, staying put is the only honest answer:
      // replacing the buffer would destroy the work and leave the reason in a
      // log the user has no reason to look at.
      if (!(await flush.current())) return;

      const request = (opening.current += 1);
      const current = () => opening.current === request;

      setHistory((previous) => {
        const next = visit(previous, path);
        historyNow.current = next;
        return next;
      });
      setLoading(true);

      try {
        const document = await readWholeDocument(vault, path);
        if (!current()) return;

        disk.current = { path, text: document.content, revision: document.revision };
        // Assigned here as well as on render: a caller that awaits `open` and
        // then reads the note back would otherwise be reading the note that
        // was on screen before it.
        noteNow.current = {
          path,
          buffer: document.content,
          dirty: false,
          // A locked note is read, not written: the CLI would refuse the save,
          // and letting someone type into a note that cannot keep it is worse
          // than not offering a caret.
          editable: !document.locked,
          locked: document.locked,
          lockedAt: document.lockedAt,
          saving: false,
          conflict: null,
          error: null,
        };
        setNote(noteNow.current);
      } catch (thrown) {
        const error =
          thrown instanceof CliFailure
            ? thrown.error
            : { code: "INTERNAL_ERROR", message: String(thrown) };
        if (!current()) return;

        record(`read ${path}`, error);
        disk.current = null;
        setNote({
          path,
          buffer: "",
          dirty: false,
          editable: false,
          locked: false,
          lockedAt: null,
          saving: false,
          conflict: null,
          error,
        });
      } finally {
        if (current()) setLoading(false);
      }
    },
    [vault, record],
  );

  openRef.current = open;

  const openLink = useCallback(
    (target: string, from: string) => {
      const resolved = resolveWikilink(target, from, paths);
      if (resolved) void open(resolved);
    },
    [open, paths],
  );

  const resolves = useCallback(
    (target: string, from: string) => resolveWikilink(target, from, paths) !== null,
    [paths],
  );

  /* Saving ---------------------------------------------------------------- */

  /**
   * Write the open note, if there is anything to write.
   *
   * Returns whether the buffer is now safely on disk, which is what lets a
   * caller decide whether it is safe to move on.
   *
   * Serialised through a promise chain rather than guarded by a boolean: a
   * guard makes a second call a no-op, and a no-op is the wrong answer when the
   * caller is asking "is my work saved?" before replacing the buffer.
   */
  const save = useCallback(async (): Promise<boolean> => {
    const run = async (): Promise<boolean> => {
      const current = noteNow.current;
      const stored = disk.current;
      if (!current || !stored || !current.editable) return true;
      // A stale closure can be holding note A while `disk.current` has already
      // moved to B. Writing A's buffer with B's revision would be a spurious
      // conflict at best.
      if (stored.path !== current.path) return true;
      if (current.buffer === stored.text) return true;
      // A conflict is the user's to resolve; retrying it would only fail again
      // and bury the choice under a second identical failure.
      if (current.conflict) return false;

      const writing = current.buffer;
      setNote((previous) => (previous ? { ...previous, saving: true, error: null } : previous));

      try {
        const result = await saveDocument(vault, current.path, writing, stored.revision);
        disk.current = { path: current.path, text: writing, revision: result.revision };
        failedAt.current = null;
        setNote((previous) =>
          previous && previous.path === current.path
            ? {
                ...previous,
                // Only clean if nothing was typed while the write was in the
                // air; otherwise the dot would clear over an unsaved edit that
                // autosave then never picks up.
                dirty: previous.buffer !== writing,
                saving: false,
                conflict: null,
              }
            : previous,
        );
        scheduleReload(vault);
        return true;
      } catch (thrown) {
        const error =
          thrown instanceof CliFailure
            ? thrown.error
            : { code: "INTERNAL_ERROR", message: String(thrown) };
        record(`save ${current.path}`, error);

        // Remember what failed, so autosave does not retry the same bytes every
        // second and a half for as long as the window is open.
        failedAt.current = { path: current.path, text: writing };

        // Locked underneath the editor — by an agent, or another window. The
        // buffer is kept, the note stops offering a caret, and the dialog says
        // what to unlock; the edits are still there to save once it is.
        if (error.code === "LOCKED") {
          setLockedError(error);
          const lockedAt =
            typeof error.details?.locked_at === "string" ? error.details.locked_at : null;
          setNote((previous) =>
            previous && previous.path === current.path
              ? { ...previous, saving: false, editable: false, locked: true, lockedAt }
              : previous,
          );
          return false;
        }

        // A conflict is not a failure to report and move on from: the buffer is
        // kept exactly as it is and the choice goes to the person.
        const conflict =
          error.code === "REVISION_CONFLICT"
            ? { theirRevision: String(error.details?.current_revision ?? "") }
            : null;
        setNote((previous) =>
          previous && previous.path === current.path
            ? { ...previous, saving: false, conflict, error: conflict ? null : error }
            : previous,
        );
        return false;
      }
    };

    // Chained, so two ⌘S presses in one tick become two sequential attempts and
    // the second finds nothing left to do — rather than two concurrent writes
    // carrying the same revision.
    const attempt = queue.current.then(run, run);
    queue.current = attempt.then(
      () => undefined,
      () => undefined,
    );
    return attempt;
  }, [vault, record, scheduleReload]);

  flush.current = save;

  /**
   * Autosave.
   *
   * Suspended by an unresolved conflict, and by a failure until the text
   * changes again — otherwise a read-only file or an unplugged vault would spawn
   * a CLI process every second and a half forever.
   */
  useEffect(() => {
    if (!note?.dirty || note.saving || note.conflict || !note.editable) return;
    if (failedAt.current?.path === note.path && failedAt.current.text === note.buffer) return;

    const timer = window.setTimeout(() => void save(), AUTOSAVE_DELAY);
    return () => window.clearTimeout(timer);
  }, [note?.dirty, note?.saving, note?.conflict, note?.editable, note?.path, note?.buffer, save]);

  // Bound in the webview too: a focused editor can swallow the key before the
  // native accelerator sees it on some WebKit builds.
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (!(event.metaKey || event.ctrlKey)) return;

      // A dialog owns the keyboard while it is open. Without this, ⌘O over the
      // Settings modal stacks a second dialog in the top layer on top of it.
      const dialogOpen = document.querySelector("dialog[open]") !== null;

      if (event.key === ",") {
        event.preventDefault();
        if (!dialogOpen) setSettingsOpen(true);
      } else if (event.key === "o") {
        event.preventDefault();
        // Toggle, so the same key that opened it puts it away.
        if (!dialogOpen) setSwitcherOpen(true);
        else setSwitcherOpen(false);
      } else if (event.key === "s") {
        // Explicit save, even though autosave is running: an editor without it
        // feels broken whether or not it needs one.
        event.preventDefault();
        void save();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [save]);

  const resolveConflict = useCallback(
    async (choice: "mine" | "theirs") => {
      const current = note;
      if (!current) return;

      if (choice === "theirs") {
        // Deliberately not through `open`: that flushes first, and flushing a
        // buffer whose revision is known stale just fails again and buries the
        // choice under a second identical error.
        setNote((previous) => (previous ? { ...previous, conflict: null } : previous));
        disk.current = null;
        await open(current.path);
        return;
      }
      // Keeping mine means re-reading to learn the revision now on disk, then
      // writing over it deliberately. It is the only overwrite path, and it
      // takes a click.
      try {
        const fresh = await readWholeDocument(vault, current.path);
        disk.current = { path: current.path, text: fresh.content, revision: fresh.revision };
        setNote((previous) => (previous ? { ...previous, conflict: null } : previous));
        await save();
      } catch (thrown) {
        if (thrown instanceof CliFailure) record(`reread ${current.path}`, thrown.error);
      }
    },
    [note, vault, open, save, record],
  );

  /* Creating -------------------------------------------------------------- */

  const createNote = useCallback(
    async (name: string) => {
      setPrompt(null);
      setActionError(null);
      if (!vault) return;
      // Beside the open note; a locked folder refuses it with `LOCKED`, which
      // `showError` puts in a dialog.
      const folder = destinationFolder(openPath);
      const path = `${folder ? `${folder}/` : ""}${name.replace(/\.md$/i, "")}.md`;

      try {
        await saveDocument(vault, path, `# ${name}\n\n`, null);
        await reload(vault);
        await open(path);
        setMode("source");
      } catch (thrown) {
        if (thrown instanceof CliFailure) {
          record(`create ${path}`, thrown.error);
          // Recording it in Diagnostics is not enough: from the workspace, a
          // refused create looks exactly like nothing happening.
          showError(thrown.error);
        }
      }
    },
    [vault, openPath, reload, open, record, showError],
  );

  const createFolderAt = useCallback(
    async (name: string) => {
      setPrompt(null);
      setActionError(null);
      if (!vault) return;
      const folder = destinationFolder(openPath);
      const path = `${folder ? `${folder}/` : ""}${name}`;

      try {
        await createFolder(vault, path);
        await reload(vault);
      } catch (thrown) {
        if (thrown instanceof CliFailure) {
          record(`create folder ${path}`, thrown.error);
          showError(thrown.error);
        }
      }
    },
    [vault, openPath, reload, record, showError],
  );

  /**
   * The notes that link to the one on screen.
   *
   * Read straight off the index, so an edit that adds or removes a link shows
   * up as soon as the index catches up rather than staying stale until the note
   * is reopened.
   */
  const mentions = useMemo(
    () => (note ? backlinksOf(index, note.path) : []),
    [index, note?.path],
  );

  /* Renaming and deleting ------------------------------------------------ */

  /** Rename by path, which is what the title field and the tree both do. */
  /**
   * Bring the open note's heading into line with its filename, or with a name
   * being committed. Reports whether anything changed.
   *
   * Nothing is written here: leaving the buffer dirty hands the write to
   * autosave and to `save`, which are the only two things in the application
   * that write a note.
   */
  const retitleOpen = useCallback((path: string, name?: string): boolean => {
    const current = noteNow.current;
    if (!current || current.path !== path || !current.editable) return false;

    const retitled = withTitle(current.buffer, name ?? titleOf(path));
    if (retitled === current.buffer) return false;

    noteNow.current = { ...current, buffer: retitled, dirty: retitled !== disk.current?.text };
    setNote(noteNow.current);
    return true;
  }, []);

  /**
   * Carry the links that pointed at a moved path over to its new one.
   *
   * Runs after the move, never instead of it. A failure does not undo the
   * rename: the move landed and the rewrite did not, and both of those are
   * true — so it is reported rather than hidden behind a rollback that would
   * itself be an unchecked write.
   *
   * What it found is returned rather than announced, because the caller has a
   * `reload` still to do and `reload` clears the banner on success. Saying it
   * here would put the one message the user must not miss on screen a moment
   * before something else wiped it.
   */
  const followMove = useCallback(
    async (target: string, from: string, to: string): Promise<MoveOutcome> => {
      try {
        const result = await relinkPaths(target, from, to);
        return {
          rewritten: result.updated.map((update) => update.path),
          shortfall: describeShortfall(result),
        };
      } catch (thrown) {
        if (thrown instanceof CliFailure) {
          record(`relink ${from}`, thrown.error);
          return { rewritten: [], shortfall: null, error: thrown.error };
        }
        throw thrown;
      }
    },
    [record],
  );

  /**
   * Say what a move could not carry. Called after the reload, never before:
   * `reload` clears the error banner on success, so anything put on screen
   * ahead of it is wiped by a refresh the user did not ask for.
   */
  const announceMove = useCallback(
    (outcome: MoveOutcome) => {
      setShortfall(outcome.shortfall);
      if (outcome.error) showError(outcome.error);
    },
    [showError],
  );

  /**
   * Do the move, and carry its inbound links if asked to.
   *
   * The single place any path in this application changes. `carryLinks` is the
   * answer to the dialog, and is false whenever there was nothing to ask about.
   */
  const performMove = useCallback(
    async ({ from, to, kind, reveal, heading }: MoveRequest, carryLinks: boolean) => {
      if (!vault) return;
      setPendingMove(null);

      try {
        // A name typed into the note's own heading moves both halves of the one
        // fact, and in this order: the write has to land on the old path,
        // before the move. Nothing was written while the dialog was up, so
        // cancelling it leaves the note exactly as it was.
        if (heading && retitleOpen(from, heading)) {
          if (!(await save())) return;
        }
        await movePath(vault, from, to);
        const outcome = carryLinks
          ? await followMove(vault, from, to)
          : { rewritten: [], shortfall: null };
        setHistory((previous) => {
          const next = forget(previous, from);
          historyNow.current = next;
          return next;
        });
        await reload(vault);
        if (kind === "document" && (reveal || openPath === from)) {
          await open(to);
          // The heading and the filename are one fact. A rename from the tree
          // moves only one of them, so the other follows here; left dirty
          // rather than written, because autosave is what writes.
          retitleOpen(to);
        } else if (openPath && outcome.rewritten.includes(openPath) && !noteNow.current?.dirty) {
          // A folder rename leaves the note on screen where it was but may have
          // rewritten links inside it. Only re-read a clean buffer: unsaved
          // edits are kept, and their next save meets `expected_revision` and
          // raises the conflict bar rather than losing anything.
          await open(openPath);
        }
        announceMove(outcome);
      } catch (thrown) {
        if (thrown instanceof CliFailure) {
          record(`move ${from}`, thrown.error);
          showError(thrown.error);
        }
      }
    },
    [vault, openPath, reload, open, record, retitleOpen, save, followMove, announceMove, showError],
  );

  /**
   * Begin a move: ask first when it would break links, otherwise just do it.
   *
   * Nothing is written before the answer. A rename that touches only the file
   * itself is not worth a dialog, and one that rewrites notes the user never
   * opened is not something to do without being asked.
   */
  const beginMove = useCallback(
    (request: MoveRequest) => {
      setActionError(null);
      setShortfall(null);
      if (!vault || request.to === request.from) return;

      const linking = backlinksUnder(index, request.from);
      if (linking.length === 0) {
        void performMove(request, false);
        return;
      }
      const lockedLinking = linking.filter(
        (path) => index?.nodes[index.indexOf.get(path) ?? -1]?.locked ?? false,
      );
      setPendingMove({ ...request, linking, lockedLinking });
    },
    [vault, index, performMove],
  );

  const renamePath = useCallback(
    (path: string, kind: "document" | "directory", name: string) => {
      setRenaming(null);
      const folder = parentOf(path);
      const suffix = kind === "document" ? ".md" : "";
      const to = `${folder ? `${folder}/` : ""}${name.replace(/\.md$/i, "")}${suffix}`;
      beginMove({ from: path, to, kind, reveal: true });
    },
    [beginMove],
  );

  /**
   * Commit a new name typed into the note's heading.
   *
   * Both halves of the one fact move, and in this order: `renamePath` ends by
   * reopening the note from disk, so a heading written into the buffer after it
   * would be written into a buffer that is about to be replaced. The save is
   * awaited for the same reason `open` flushes one — the write has to land on
   * the old path, before the move.
   */
  const retitle = useCallback(
    async (name: string) => {
      const current = noteNow.current;
      if (!current) return;

      // Only the heading differs, so there is no move to ask about.
      if (titleOf(current.path) === name) {
        if (current.editable && retitleOpen(current.path, name)) await save();
        return;
      }

      const folder = parentOf(current.path);
      const to = `${folder ? `${folder}/` : ""}${name.replace(/\.md$/i, "")}.md`;
      beginMove({
        from: current.path,
        to,
        kind: "document",
        reveal: true,
        heading: current.editable ? name : undefined,
      });
    },
    [retitleOpen, save, beginMove],
  );

  const rename = useCallback(
    (node: TreeNode, name: string) => void renamePath(node.path, node.kind, name),
    [renamePath],
  );

  /** Drop a note or folder into another folder. */
  const moveInto = useCallback(
    (path: string, folder: string) => {
      const name = baseName(path);
      const to = folder ? `${folder}/${name}` : name;
      // Already there, or dropped onto itself.
      if (to === path) return;
      // A folder cannot be moved inside itself; the CLI refuses it, but saying
      // nothing at all reads better than an error for something the user has
      // already seen is impossible.
      if (folder === path || folder.startsWith(`${path}/`)) return;

      // A drag changes a path just as a rename does, and a path-shaped link
      // breaks either way, so it asks the same question. Only `.md` files are
      // notes, which is the same rule the tree sorts by.
      beginMove({ from: path, to, kind: path.endsWith(".md") ? "document" : "directory", reveal: false });
    },
    [beginMove],
  );

  const remove = useCallback(
    async (node: TreeNode) => {
      setDeleting(null);
      setActionError(null);
      if (!vault) return;

      try {
        await deletePath(vault, node.path);
        setHistory((previous) => {
          const next = forget(previous, node.path);
          historyNow.current = next;
          return next;
        });
        // The open note may have just been the one that went; drop it rather
        // than leaving a buffer pointing at a file that is no longer there.
        if (openPath === node.path || openPath?.startsWith(`${node.path}/`)) {
          setNote(null);
          disk.current = null;
        }
        await reload(vault);
      } catch (thrown) {
        if (thrown instanceof CliFailure) {
          record(`delete ${node.path}`, thrown.error);
          showError(thrown.error);
        }
      }
    },
    [vault, openPath, reload, record, showError],
  );

  /** Folders whose own state is locked, and the root as `""` when it is. */
  const lockedDirs = useMemo(() => {
    const found = new Set(
      listing.filter((entry) => entry.kind === "directory" && entry.locked).map((entry) => entry.path),
    );
    if (rootLocked) found.add("");
    return found;
  }, [listing, rootLocked]);

  /** Set while an unlock is waiting for a person to answer. */
  const unlockPending = useRef(false);

  /**
   * Lock or unlock one path, or the whole vault when `path` is null.
   *
   * Unsaved edits are written first: locking a dirty note would otherwise strand
   * them in a buffer that can no longer be saved. The open note's editability
   * then follows the new state without reopening it, so the caret and the undo
   * history stay where they were.
   */
  const toggleLock = useCallback(
    async (path: string | null, locked: boolean) => {
      if (!vault) return;
      // An unlock waits on Touch ID or the password (SPEC §6); a second click
      // while the prompt is up must not stack a second prompt behind it.
      if (unlockPending.current) return;
      setActionError(null);
      if (!(await flush.current())) return;

      if (locked) unlockPending.current = true;
      try {
        if (locked) await unlockPath(vault, path);
        else await lockPath(vault, path);
      } catch (thrown) {
        if (thrown instanceof CliFailure) {
          record(`${locked ? "unlock" : "lock"} ${path ?? "vault"}`, thrown.error);
          // The person said no; the note simply stays locked. Anything else —
          // no way to ask, no answer in time — is worth a dialog.
          const cancelled =
            thrown.error.code === "NOT_CONFIRMED" && thrown.error.details?.reason === "cancelled";
          if (!cancelled) showError(thrown.error);
        }
        return;
      } finally {
        unlockPending.current = false;
      }

      await reload(vault);

      const current = noteNow.current;
      if (!current || current.error) return;
      try {
        const state = await readLockState(vault, current.path);
        // Edits a refused write left in the buffer can go out now; autosave
        // would otherwise skip them as bytes that already failed once.
        if (!state.locked && failedAt.current?.path === current.path) failedAt.current = null;
        setNote((previous) =>
          previous && previous.path === current.path
            ? {
                ...previous,
                locked: state.locked,
                lockedAt: state.lockedAt,
                editable: !state.locked,
              }
            : previous,
        );
      } catch (thrown) {
        if (thrown instanceof CliFailure) record(`read ${current.path}`, thrown.error);
      }
    },
    [vault, reload, record, showError],
  );

  /**
   * What a right-click offers.
   *
   * A locked path cannot be renamed or deleted, nor can a folder with anything
   * locked inside it, nor anything inside a locked folder — the CLI would
   * refuse, and offering the option would only produce an error. Locking is
   * always offered, since it is how the rest comes back.
   */
  const menuItems = useCallback(
    (node: TreeNode): MenuItem[] => {
      const pinned = containsLocked(node) || lockedDirs.has(parentOf(node.path));
      return [
        ...(pinned
          ? []
          : [
              { label: "Rename…", onSelect: () => setRenaming(node) },
              { label: "Delete…", onSelect: () => setDeleting(node), destructive: true },
            ]),
        {
          label: node.locked ? "Unlock" : "Lock",
          onSelect: () => void toggleLock(node.path, node.locked),
        },
      ];
    },
    [lockedDirs, toggleLock],
  );

  /* Render ---------------------------------------------------------------- */

  // Memoised on the index, not rebuilt each render: `GraphPane` keys its
  // simulation off this object's identity, so a new literal on every keystroke
  // would throw away every node position while someone was typing.
  const graphData: LinkGraphData | null = useMemo(
    () =>
      index
        ? {
            nodes: index.nodes,
            edges: index.edges,
            unresolved: index.unresolved,
            truncated: index.truncated,
          }
        : null,
    [index],
  );

  const resolvedTheme = resolveTheme(theme, prefersDark);

  return (
    // Draggable so the window still moves when there is no vault and neither
    // toolbar is mounted; the overlaid title bar left nothing else to grab.
    <div className="app" data-tauri-drag-region>
      {/* No role here: `Failure` is already the alert, and nesting two makes a
          screen reader announce it twice. */}
      {actionError ? (
        <div className="app__banner" data-tauri-drag-region="deep">
          <Failure error={actionError} />
          <Button onClick={() => setActionError(null)}>Dismiss</Button>
        </div>
      ) : null}



      {vault ? (
        <Workspace
          widths={panes}
          onResize={(side, width) => setPanes((previous) => ({ ...previous, [side]: width }))}
          left={
            <Explorer
              entries={listing}
              openPath={openPath}
              truncated={truncated}
              onOpen={(path) => void open(path)}
              onNewNote={() => setPrompt("note")}
              onNewFolder={() => setPrompt("folder")}
              onContextMenu={(node, at) =>
                menuItems(node).length > 0 ? setMenu({ node, ...at }) : undefined
              }
              onMove={(path, folder) => void moveInto(path, folder)}
              rootLocked={rootLocked}
              onToggleVaultLock={() => void toggleLock(null, rootLocked)}
            />
          }
          centre={
            <NoteView
              note={note}
              loading={loading}
              // The path being opened, which during a slow read is not yet the
              // path on screen.
              openingPath={openPath}
              mode={mode}
              // Keyed to the note actually on screen, not to the one being
              // opened: during a slow read the two differ, and showing one
              // note's links under another note's body is just wrong.
              mentions={mentions}
              canBack={canGoBack(history)}
              canForward={canGoForward(history)}
              renderMermaid={(code) => (
                <Mermaid code={code} theme={resolvedTheme} accent={accents[resolvedTheme]} />
              )}
              onModeChange={setMode}
              onBack={() => step(goBack)}
              onForward={() => step(goForward)}
              onChange={(text) =>
                setNote((previous) =>
                  previous
                    ? { ...previous, buffer: text, dirty: text !== disk.current?.text }
                    : previous,
                )
              }
              onSave={() => void save()}
              onOpen={(path) => void open(path)}
              onOpenLink={openLink}
              resolves={resolves}
              onResolveConflict={(choice) => void resolveConflict(choice)}
              // The heading and the filename are the same fact, so editing one
              // moves both. A locked note cannot be moved, so it is not offered.
              onRename={note && !note.locked ? (name) => void retitle(name) : null}
              onToggleLock={
                note && !note.error ? () => void toggleLock(note.path, note.locked) : null
              }
            />
          }
          right={
            <GraphPane data={graphData} activePath={openPath} onOpen={(path) => void open(path)} />
          }
        />
      ) : (
        <p className="empty">
          {forgotten ? (
            <>
              The folder for “{forgotten}” was deleted, so Heimdall has forgotten it.{" "}
            </>
          ) : null}
          No vault yet. Open <strong>Heimdall → Settings…</strong> to create one or choose an
          existing folder.
        </p>
      )}

      {menu ? (
        <ContextMenu
          x={menu.x}
          y={menu.y}
          items={menuItems(menu.node)}
          onClose={() => setMenu(null)}
        />
      ) : null}

      <Prompt
        title={renaming ? "Rename" : prompt === "folder" ? "Create folder" : "Create note"}
        label={
          renaming
            ? renaming.kind === "directory"
              ? "Folder name"
              : "Note name"
            : prompt === "folder"
              ? "Folder name"
              : "Note name"
        }
        initial={renaming ? titleOf(baseName(renaming.path)) : prompt === "folder" ? "notes" : "untitled"}
        note={undefined}
        open={prompt !== null || renaming !== null}
        onSubmit={(name) => {
          if (renaming) rename(renaming, name);
          else void (prompt === "folder" ? createFolderAt(name) : createNote(name));
        }}
        onCancel={() => {
          setPrompt(null);
          setRenaming(null);
        }}
      />

      {/*
        Every move that would break inbound links stops here, whether it came
        from the tree, the note's own heading, or a drag. Nothing is written
        until one of these buttons is pressed, so cancelling leaves the vault
        untouched — and the rewrite is never something that merely happened.
      */}
      <Confirm
        title="Rename"
        open={pendingMove !== null}
        confirmLabel="Update links"
        secondary={{
          label: "Rename only",
          onSelect: () => pendingMove && void performMove(pendingMove, false),
        }}
        onConfirm={() => pendingMove && void performMove(pendingMove, true)}
        onCancel={() => setPendingMove(null)}
      >
        <p>
          Rename <strong>{pendingMove ? baseName(pendingMove.from) : ""}</strong> to{" "}
          <strong>{pendingMove ? baseName(pendingMove.to) : ""}</strong>?
        </p>
        <p className="muted">
          {pendingMove ? plural(pendingMove.linking.length, "note") : ""} link here.{" "}
          <strong>Update links</strong> rewrites them to the new name.{" "}
          <strong>Rename only</strong> leaves them pointing at the old one.
        </p>
        {pendingMove ? (
          <ul className="confirm__list">
            {pendingMove.linking.slice(0, 8).map((path) => (
              <li key={path}>{path}</li>
            ))}
            {pendingMove.linking.length > 8 ? (
              <li className="muted">and {pendingMove.linking.length - 8} more</li>
            ) : null}
          </ul>
        ) : null}
        {pendingMove && pendingMove.lockedLinking.length > 0 ? (
          <p className="muted">
            {pendingMove.lockedLinking.length} of these{" "}
            {pendingMove.lockedLinking.length === 1 ? "is" : "are"} locked and will be left
            unchanged, still pointing at the old name.
          </p>
        ) : null}
      </Confirm>

      {/*
        Only shortfalls are reported, and they are reported in a dialog rather
        than a banner: a refresh clears the banner, and this is the one message
        that must survive until it has been read.
      */}
      <Modal title="Some links were left behind" open={shortfall !== null} onClose={() => setShortfall(null)}>
        <div className="confirm__frame">
          <p>{shortfall}</p>
          <div className="row confirm__actions">
            <Button primary onClick={() => setShortfall(null)}>
              OK
            </Button>
          </div>
        </div>
      </Modal>

      {/* A refusal because something is locked. A dialog for the same reason
          the shortfall is one: the banner does not survive the next reload. */}
      <Modal title="Locked" open={lockedError !== null} onClose={() => setLockedError(null)}>
        <div className="confirm__frame">
          <p>{lockedError ? describeLocked(lockedError) : ""}</p>
          <div className="row confirm__actions">
            <Button primary onClick={() => setLockedError(null)}>
              OK
            </Button>
          </div>
        </div>
      </Modal>

      <Confirm
        title="Delete"
        open={deleting !== null}
        confirmLabel="Move to trash"
        onConfirm={() => deleting && void remove(deleting)}
        onCancel={() => setDeleting(null)}
      >
        <p>
          Move <strong>{deleting ? baseName(deleting.path) : ""}</strong> to the vault&rsquo;s
          trash? It stays on disk in <code>.trash/</code> and can be put back from there.
        </p>
        {deleting && backlinksUnder(index, deleting.path).length > 0 ? (
          <p className="muted">
            {plural(backlinksUnder(index, deleting.path).length, "note")} link to it; those links
            will stop resolving. Deletion has no new name to point them at.
          </p>
        ) : null}
      </Confirm>

      <QuickSwitcher
        open={switcherOpen}
        paths={paths}
        onOpen={(path) => void open(path)}
        onClose={() => setSwitcherOpen(false)}
      />

      <SettingsModal
        open={settingsOpen}
        vault={vault}
        status={status}
        failures={failures}
        theme={theme}
        onThemeChange={setTheme}
        accents={accents}
        onAccentsChange={setAccents}
        onVaultChange={chooseVault}
        onRefresh={refreshStatus}
        onClose={() => setSettingsOpen(false)}
      />
    </div>
  );
}

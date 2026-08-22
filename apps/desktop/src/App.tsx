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

import { cliStatus } from "./api/cli";
import {
  CliFailure,
  createFolder,
  deletePath,
  listAllDocuments,
  movePath,
  readWholeDocument,
  saveDocument,
} from "./api/documents";
import {
  backlinksOf,
  inboundLinkCount,
  loadVaultIndex,
  type VaultIndex,
} from "./api/index-graph";
import { baseName, isProtected, parentOf, sourceOf, titleOf } from "./api/source";
import type { CliStatus, DomainError, LinkGraphData } from "./api/types";
import { Button, Failure } from "./components";
import { Explorer } from "./features/explorer/Explorer";
import type { TreeInput, TreeNode } from "./features/explorer/tree";
import { GraphPane } from "./features/graph/GraphPane";
import { Mermaid } from "./features/note/Mermaid";
import { NoteView, type OpenNote, type ViewMode } from "./features/note/NoteView";
import type { RecordedFailure } from "./features/settings/Diagnostics";
import { SettingsModal } from "./features/settings/SettingsModal";
import { QuickSwitcher } from "./features/switcher/QuickSwitcher";
import { Prompt } from "./components/Prompt";
import { ContextMenu, type MenuItem } from "./components/ContextMenu";
import { Confirm } from "./components/Confirm";
import { Workspace } from "./features/workspace/Workspace";
import { DEFAULT_PANES, isPaneWidths, type PaneWidths } from "./features/workspace/panes";
import { readPref, writePref } from "./state/prefs";
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
 * Where a new note or folder goes.
 *
 * Beside whatever is open, unless that is inside `aios/` — the protected tree
 * has no create operation, so putting it there would only earn a refusal.
 */
function destinationFolder(openPath: string | null): string {
  if (!openPath) return "";
  const folder = parentOf(openPath);
  return isProtected(folder) ? "" : folder;
}

/** The event the Rust menu emits for Heimdall → Settings…. */
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
  const [failures, setFailures] = useState<RecordedFailure[]>([]);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [switcherOpen, setSwitcherOpen] = useState(false);
  const [prompt, setPrompt] = useState<"note" | "folder" | null>(null);
  const [menu, setMenu] = useState<{ node: TreeNode; x: number; y: number } | null>(null);
  const [renaming, setRenaming] = useState<TreeNode | null>(null);
  const [deleting, setDeleting] = useState<TreeNode | null>(null);
  const [actionError, setActionError] = useState<DomainError | null>(null);

  const [theme, setTheme] = useState<ThemePreference>(() =>
    readPref("theme", "system" as ThemePreference, isThemePreference),
  );
  const [panes, setPanes] = useState<PaneWidths>(() =>
    readPref("panes", DEFAULT_PANES, isPaneWidths),
  );
  const [prefersDark, setPrefersDark] = useState(systemPrefersDark);

  const [index, setIndex] = useState<VaultIndex | null>(null);
  const [listing, setListing] = useState<TreeInput[]>([]);
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
        // The tree comes from both: the listing supplies ordinary folders,
        // including empty ones, and the index is the only thing that sees the
        // protected tree at all.
        setListing([
          ...nextListing.entries.map((entry) => ({ path: entry.path, kind: entry.kind })),
          ...nextIndex.nodes
            .filter((node) => isProtected(node.path))
            .map((node) => ({ path: node.path, kind: "document" as const })),
        ]);
        setActionError(null);
      } catch (thrown) {
        if (thrown instanceof CliFailure) {
          record("link-graph", thrown.error);
          // A vault that has moved, or was never initialised, otherwise leaves
          // the workspace showing an empty tree and "Building the graph…"
          // forever with the reason buried in Diagnostics.
          setActionError(thrown.error);
          setIndex(null);
          setListing([]);
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
   * Nothing watches the filesystem, so a note changed in Obsidian or by an
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
      // Entries are the one thing the editor reads but does not offer to write.
      // They are created by an agent and their frontmatter is Heimdall's (SPEC
      // §6); editing the body is possible, but doing it by accident while
      // reading a summary is not what anyone wants.
      const source = sourceOf(path);
      const editable = source !== "entry-conversation" && source !== "entry-notification";

      try {
        const document = await readWholeDocument(vault, path);
        if (!current()) return;

        disk.current = { path, text: document.content, revision: document.revision };
        setNote({
          path,
          buffer: document.content,
          dirty: false,
          editable,
          saving: false,
          conflict: null,
          error: null,
        });
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
      // Beside the open note, unless that note is in the protected tree — which
      // has no create operation, so the vault root is the honest default.
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
          setActionError(thrown.error);
        }
      }
    },
    [vault, openPath, reload, open, record],
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
          setActionError(thrown.error);
        }
      }
    },
    [vault, openPath, reload, record],
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
  const renamePath = useCallback(
    async (path: string, kind: "document" | "directory", name: string) => {
      setRenaming(null);
      setActionError(null);
      if (!vault) return;

      const folder = parentOf(path);
      const suffix = kind === "document" ? ".md" : "";
      const to = `${folder ? `${folder}/` : ""}${name.replace(/\.md$/i, "")}${suffix}`;
      if (to === path) return;

      try {
        await movePath(vault, path, to);
        setHistory((previous) => {
          const next = forget(previous, path);
          historyNow.current = next;
          return next;
        });
        await reload(vault);
        if (kind === "document") await open(to);
      } catch (thrown) {
        if (thrown instanceof CliFailure) {
          record(`rename ${path}`, thrown.error);
          setActionError(thrown.error);
        }
      }
    },
    [vault, reload, open, record],
  );

  const rename = useCallback(
    (node: TreeNode, name: string) => void renamePath(node.path, node.kind, name),
    [renamePath],
  );

  /** Drop a note or folder into another folder. */
  const moveInto = useCallback(
    async (path: string, folder: string) => {
      setActionError(null);
      if (!vault) return;

      const name = baseName(path);
      const to = folder ? `${folder}/${name}` : name;
      // Already there, or dropped onto itself.
      if (to === path) return;
      // A folder cannot be moved inside itself; the CLI refuses it, but saying
      // nothing at all reads better than an error for something the user has
      // already seen is impossible.
      if (folder === path || folder.startsWith(`${path}/`)) return;

      try {
        await movePath(vault, path, to);
        setHistory((previous) => {
          const next = forget(previous, path);
          historyNow.current = next;
          return next;
        });
        await reload(vault);
        if (openPath === path) await open(to);
      } catch (thrown) {
        if (thrown instanceof CliFailure) {
          record(`move ${path}`, thrown.error);
          setActionError(thrown.error);
        }
      }
    },
    [vault, openPath, reload, open, record],
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
          setActionError(thrown.error);
        }
      }
    },
    [vault, openPath, reload, record],
  );

  /**
   * What a right-click offers.
   *
   * The protected tree is Heimdall's structure, so it is browsable but not
   * rearrangeable — the CLI would refuse, and offering the option would only
   * produce an error.
   */
  const menuItems = useCallback(
    (node: TreeNode): MenuItem[] => {
      if (node.inAios) return [];
      return [
        { label: "Rename…", onSelect: () => setRenaming(node) },
        { label: "Delete…", onSelect: () => setDeleting(node), destructive: true },
      ];
    },
    [],
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
    <div className="app">
      {/* No role here: `Failure` is already the alert, and nesting two makes a
          screen reader announce it twice. */}
      {actionError ? (
        <div className="app__banner">
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
              renderMermaid={(code) => <Mermaid code={code} theme={resolvedTheme} />}
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
              // The title is the filename, so editing one renames the other.
              // Protected content is Heimdall's to name, so it is not offered.
              onRename={
                note && !isProtected(note.path)
                  ? (name) => void renamePath(note.path, "document", name)
                  : null
              }
            />
          }
          right={
            <GraphPane data={graphData} activePath={openPath} onOpen={(path) => void open(path)} />
          }
        />
      ) : (
        <p className="empty">
          No vault yet. Open <strong>Heimdall → Settings…</strong> to create one or choose an
          existing Obsidian vault.
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
        note={
          renaming && inboundLinkCount(index, renaming.path) > 0
            ? `${inboundLinkCount(index, renaming.path)} note(s) link here. Renaming does not rewrite those links.`
            : undefined
        }
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
        {deleting && inboundLinkCount(index, deleting.path) > 0 ? (
          <p className="muted">
            {inboundLinkCount(index, deleting.path)} note(s) link to it; those links will stop
            resolving.
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
        onVaultChange={chooseVault}
        onRefresh={refreshStatus}
        onClose={() => setSettingsOpen(false)}
      />
    </div>
  );
}

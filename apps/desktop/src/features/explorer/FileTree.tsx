/**
 * The vault's file tree.
 *
 * An `aria-tree`, so it is navigable by keyboard and announced as a tree rather
 * than as a pile of buttons.
 *
 * Moving a note is a **pointer-event drag, not an HTML5 one**. HTML5
 * drag-and-drop inside a WKWebView is a minefield — Tauri installs its own
 * OS-level drag handler over the webview, `-webkit-user-drag` is not inherited
 * so a grab on a child element never starts a drag, and a `<button>` will not
 * begin one at all. Pointer events have none of that: they are the same
 * mechanism the graph pane already drags nodes with, and they work. The
 * stylesheet turns WebKit's own drag off on the rows, so the two cannot both
 * claim the press.
 *
 * The drag itself renders as little as it can. The cursor label is moved by
 * writing to its own element, and React state changes only when the folder a
 * drop would land in changes — re-rendering the whole tree on every pointer
 * event is what made the earlier version stutter.
 */

import { useEffect, useMemo, useRef, useState } from "react";

import { parentOf } from "../../api/source";
import { IconChevronDown, IconChevronRight, IconLock } from "../../components/icons";
import { containsLocked, lockedFolders, type TreeNode } from "./tree";

interface FileTreeProps {
  nodes: TreeNode[];
  openPath: string | null;
  expanded: ReadonlySet<string>;
  onToggle: (path: string) => void;
  onOpen: (path: string) => void;
  onContextMenu: (node: TreeNode, at: { x: number; y: number }) => void;
  /** Move `path` into `folder`. `folder` is `""` for the vault root. */
  onMove: (path: string, folder: string) => void;
  /** Whether the vault root is locked, which makes it no place to drop. */
  rootLocked?: boolean;
}

/** How far the pointer travels before a press becomes a drag. */
const DRAG_SLOP = 5;

/**
 * What the tree re-renders for during a drag.
 *
 * Only the row being dragged and the folder a drop would land in: the cursor
 * position is not here, because the label that follows the cursor is moved
 * directly rather than through React.
 */
interface Dragging {
  path: string;
  label: string;
  /** The folder a drop would land in, or null when there is no valid target. */
  over: string | null;
}

export function FileTree({
  nodes,
  openPath,
  expanded,
  onToggle,
  onOpen,
  onContextMenu,
  onMove,
  rootLocked = false,
}: FileTreeProps) {
  const [drag, setDrag] = useState<Dragging | null>(null);
  /**
   * Whether a folder is read-only, so nothing can be dropped into it or dragged
   * out of it. Held in a ref for the window listeners, which are installed once.
   */
  const locked = useMemo(() => lockedFolders(nodes), [nodes]);
  const folderLocked = useRef<(folder: string) => boolean>(() => false);
  folderLocked.current = (folder) => (folder === "" ? rootLocked : locked.has(folder));
  const candidate = useRef<{ path: string; label: string; x: number; y: number } | null>(null);
  /** Where the cursor is, for the label that follows it. */
  const pointer = useRef({ x: 0, y: 0 });
  const ghost = useRef<HTMLDivElement | null>(null);
  /**
   * Whether the press that just ended was a drag.
   *
   * `click` fires after `pointerup`, by which point the drag state has already
   * been cleared — so without remembering it here, every drag would also open
   * the note it started from.
   */
  const wasDrag = useRef(false);
  /** Kept fresh for the window listeners, which are installed once. */
  const onMoveNow = useRef(onMove);
  onMoveNow.current = onMove;

  /**
   * Track the pointer on the window rather than on the row.
   *
   * Pointer capture is the tidier mechanism and it is what the graph pane uses,
   * but here a row can re-render mid-drag and the capture does not survive it —
   * the drag would start, move once, and freeze. The window always sees the
   * pointer.
   */
  useEffect(() => {
    const move = (event: PointerEvent) => {
      const start = candidate.current;
      if (!start) return;
      if (!wasDrag.current && Math.hypot(event.clientX - start.x, event.clientY - start.y) < DRAG_SLOP) {
        return;
      }

      wasDrag.current = true;
      // Moved by hand: the label follows the cursor at pointer rate, and a
      // React render per pointer event to move a small box is what made the
      // drag feel like it was catching on something.
      pointer.current = { x: event.clientX, y: event.clientY };
      place(ghost.current, pointer.current);

      const over = landingAt(event.clientX, event.clientY, start.path, folderLocked.current);
      setDrag((previous) =>
        previous && previous.path === start.path && previous.over === over
          ? previous
          : { path: start.path, label: start.label, over },
      );
    };

    const end = (event: PointerEvent) => {
      const start = candidate.current;
      candidate.current = null;
      if (!start || !wasDrag.current) {
        setDrag(null);
        return;
      }

      const folder = landingAt(event.clientX, event.clientY, start.path, folderLocked.current);
      setDrag(null);
      if (folder === null) return;

      const name = start.path.slice(start.path.lastIndexOf("/") + 1);
      const to = folder ? `${folder}/${name}` : name;
      // A drop back where it started is not a move.
      if (to !== start.path) onMoveNow.current(start.path, folder);
    };

    // A cancelled gesture is not a drop: the pointer was taken away rather than
    // released, and committing a move on it would file a note somewhere nobody
    // chose.
    const abandon = () => {
      candidate.current = null;
      setDrag(null);
    };

    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", end);
    window.addEventListener("pointercancel", abandon);
    return () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", end);
      window.removeEventListener("pointercancel", abandon);
    };
  }, []);

  return (
    <>
      <ul
        // The root of the tree is also the vault root as a drop target: the
        // whole pane below the last row is `data-path=""`, so dragging a note
        // out of a folder has somewhere to land. Without it the only way out of
        // a folder was a rename.
        className={`tree tree--root${drag?.over === "" ? " tree--drop" : ""}`}
        role="tree"
        aria-label="Vault"
        data-path=""
        data-kind="directory"
        data-locked={rootLocked ? "true" : "false"}
      >
        {nodes.map((node) => (
          <TreeRow
            key={node.path}
            node={node}
            depth={0}
            openPath={openPath}
            expanded={expanded}
            onToggle={onToggle}
            onOpen={onOpen}
            onContextMenu={onContextMenu}
            onMove={onMove}
            dragging={drag}
            wasDragged={wasDrag}
            onPress={(pressed, event) => {
              wasDrag.current = false;
              // A locked path cannot move, nor can a folder with anything
              // locked inside it, nor anything out of a locked folder — the
              // CLI refuses all three, so the drag never starts.
              if (containsLocked(pressed) || folderLocked.current(parentOf(pressed.path))) return;
              candidate.current = {
                path: pressed.path,
                label: pressed.label,
                x: event.clientX,
                y: event.clientY,
              };
            }}
          />
        ))}
      </ul>

      {drag ? (
        // Positioned by the callback as well as by the pointer handler: the
        // element does not exist yet on the event that creates it, and without
        // this the label appears in the top-left corner for one frame.
        <div
          className="tree__ghost"
          ref={(element) => {
            ghost.current = element;
            place(element, pointer.current);
          }}
        >
          {drag.label}
          {drag.over === null ? <span className="tree__ghost-note"> — nowhere to drop</span> : null}
        </div>
      ) : null}
    </>
  );
}

/** Put the cursor label at a point, just clear of the cursor itself. */
function place(element: HTMLElement | null, at: { x: number; y: number }): void {
  if (element) element.style.transform = `translate(${at.x + 12}px, ${at.y + 8}px)`;
}

/**
 * The folder a drop at this point would land in.
 *
 * Read off the row under the cursor: onto a folder means into it, onto a note
 * means into the folder that note lives in, and onto the tree itself — the
 * empty space below the rows — means the vault root. `null` when the point is
 * over nothing droppable: outside the tree, a locked folder (or a note inside
 * one), or the dragged folder itself.
 */
function landingAt(
  x: number,
  y: number,
  dragged: string,
  folderLocked: (folder: string) => boolean,
): string | null {
  const row = (document.elementFromPoint(x, y) as HTMLElement | null)?.closest<HTMLElement>(
    "[data-path]",
  );
  if (!row) return null;

  const path = row.dataset.path ?? "";
  // `parentOf` rather than slicing at `lastIndexOf("/")`: that returns -1 for a
  // note at the vault root, and `slice(0, -1)` then quietly drops the last
  // character — a drop near `Sam.md` tried to move into a folder called "Sam.m".
  const folder = row.dataset.kind === "directory" ? path : parentOf(path);

  // Dropping a folder inside itself would detach the subtree from the vault.
  if (folder === dragged || folder.startsWith(`${dragged}/`)) return null;
  // A locked folder takes nothing new, whichever row in it was pointed at.
  if (folderLocked(folder)) return null;
  return folder;
}

interface RowProps extends Omit<FileTreeProps, "nodes" | "rootLocked"> {
  node: TreeNode;
  depth: number;
  dragging: Dragging | null;
  wasDragged: React.RefObject<boolean>;
  onPress: (node: TreeNode, event: React.PointerEvent<HTMLDivElement>) => void;
}

function TreeRow({
  node,
  depth,
  openPath,
  expanded,
  onToggle,
  onOpen,
  onContextMenu,
  onMove,
  dragging,
  wasDragged,
  onPress,
}: RowProps) {
  const isFolder = node.kind === "directory";
  const isOpen = expanded.has(node.path);
  const isCurrent = node.path === openPath;

  // Only the folder itself is marked, not every note that happens to live in
  // it: lighting up a whole folder's worth of rows says less about where the
  // drop lands than one outline does.
  const isDropTarget = isFolder && dragging !== null && dragging.over === node.path;

  const rowClass = [
    isFolder ? "tree__row tree__row--folder" : "tree__row",
    isCurrent ? "tree__row--current" : "",
    isDropTarget ? "tree__row--drop" : "",
    dragging?.path === node.path ? "tree__row--dragging" : "",
  ]
    .filter(Boolean)
    .join(" ");

  return (
    <li role="none">
      <div
        role="treeitem"
        tabIndex={0}
        data-path={node.path}
        data-kind={node.kind}
        data-locked={node.locked ? "true" : "false"}
        aria-level={depth + 1}
        aria-expanded={isFolder ? isOpen : undefined}
        aria-current={isCurrent ? "page" : undefined}
        aria-grabbed={dragging?.path === node.path ? true : undefined}
        className={rowClass}
        // Indentation is inline because it is data, not style: it comes from
        // how deep the path is, which no stylesheet can know.
        style={{ paddingLeft: `${8 + depth * 14}px` }}
        onPointerDown={(event) => {
          // Only a secondary button is rejected. Testing `!== 0` would also
          // reject an event whose `button` is undefined, which is what jsdom
          // produces — the drag would then be untestable.
          if (event.button > 0) return;
          onPress(node, event);
        }}
        onClick={() => {
          // A press that turned into a drag is not also a click.
          if (wasDragged.current) {
            wasDragged.current = false;
            return;
          }
          if (isFolder) onToggle(node.path);
          else onOpen(node.path);
        }}
        onKeyDown={(event) => {
          if (event.key !== "Enter" && event.key !== " ") return;
          event.preventDefault();
          if (isFolder) onToggle(node.path);
          else onOpen(node.path);
        }}
        onContextMenu={(event) => {
          event.preventDefault();
          onContextMenu(node, { x: event.clientX, y: event.clientY });
        }}
      >
        <span className="tree__twist">
          {isFolder ? isOpen ? <IconChevronDown size={12} /> : <IconChevronRight size={12} /> : null}
        </span>
        <span className="tree__label">{node.label}</span>
        {node.locked ? (
          <span className="tree__lock" title="Locked">
            <IconLock size={11} />
          </span>
        ) : null}
      </div>

      {isFolder && isOpen && node.children.length > 0 ? (
        <ul className="tree" role="group">
          {node.children.map((child) => (
            <TreeRow
              key={child.path}
              node={child}
              depth={depth + 1}
              openPath={openPath}
              expanded={expanded}
              onToggle={onToggle}
              onOpen={onOpen}
              onContextMenu={onContextMenu}
              onMove={onMove}
              dragging={dragging}
              wasDragged={wasDragged}
              onPress={onPress}
            />
          ))}
        </ul>
      ) : null}
    </li>
  );
}

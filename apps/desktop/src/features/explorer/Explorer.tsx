/**
 * The left pane: a toolbar and the vault's tree.
 */

import { useEffect, useMemo, useState } from "react";

import {
  IconCollapse,
  IconNewFolder,
  IconNewNote,
  IconSort,
} from "../../components/icons";
import { FileTree } from "./FileTree";
import {
  ancestorsOf,
  buildTree,
  folderPaths,
  type SortOrder,
  type TreeInput,
  type TreeNode,
} from "./tree";

interface ExplorerProps {
  entries: TreeInput[];
  openPath: string | null;
  /** Set when the listing stopped early, so the tree can say so. */
  truncated?: boolean;
  onOpen: (path: string) => void;
  onNewNote: () => void;
  onNewFolder: () => void;
  onContextMenu: (node: TreeNode, at: { x: number; y: number }) => void;
  onMove: (path: string, folder: string) => void;
}

export function Explorer({
  entries,
  openPath,
  truncated,
  onOpen,
  onNewNote,
  onNewFolder,
  onContextMenu,
  onMove,
}: ExplorerProps) {
  const [order, setOrder] = useState<SortOrder>("name");
  const [collapsed, setCollapsed] = useState<ReadonlySet<string>>(new Set());

  // Reveal whatever was opened. A note reached from the quick switcher, a
  // wikilink, a backlink or the graph is often in a folder the user closed, and
  // highlighting a row inside a collapsed folder tells them nothing about where
  // they are.
  useEffect(() => {
    if (!openPath) return;
    setCollapsed((previous) => {
      const ancestors = ancestorsOf(openPath);
      if (!ancestors.some((folder) => previous.has(folder))) return previous;

      const next = new Set(previous);
      for (const folder of ancestors) next.delete(folder);
      return next;
    });
  }, [openPath]);

  const nodes = useMemo(() => buildTree(entries, order), [entries, order]);
  // Folders start open, matching the screenshots, so the set tracks what has
  // been closed rather than what has been opened.
  const expanded = useMemo(
    () => new Set(folderPaths(nodes).filter((path) => !collapsed.has(path))),
    [nodes, collapsed],
  );

  const toggle = (path: string) =>
    setCollapsed((previous) => {
      const next = new Set(previous);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });

  const allCollapsed = expanded.size === 0;

  return (
    <div className="explorer">
      <div className="explorer__toolbar">
        <button type="button" className="icon-button" title="New note" aria-label="New note" onClick={onNewNote}>
          <IconNewNote />
        </button>
        <button
          type="button"
          className="icon-button"
          title="New folder"
          aria-label="New folder"
          onClick={onNewFolder}
        >
          <IconNewFolder />
        </button>
        <button
          type="button"
          className="icon-button"
          title={order === "name" ? "Sort Z to A" : "Sort A to Z"}
          aria-label="Change sort order"
          onClick={() => setOrder(order === "name" ? "name-desc" : "name")}
        >
          <IconSort />
        </button>
        <button
          type="button"
          className="icon-button"
          title={allCollapsed ? "Expand all" : "Collapse all"}
          aria-label={allCollapsed ? "Expand all" : "Collapse all"}
          onClick={() => setCollapsed(allCollapsed ? new Set() : new Set(folderPaths(nodes)))}
        >
          <IconCollapse />
        </button>
      </div>

      <div className="explorer__scroll">
        {nodes.length === 0 ? (
          <p className="tree__note">This vault has no notes yet.</p>
        ) : (
          <FileTree
            nodes={nodes}
            openPath={openPath}
            expanded={expanded}
            onToggle={toggle}
            onOpen={onOpen}
            onContextMenu={onContextMenu}
            onMove={onMove}
          />
        )}
        {truncated ? (
          <p className="tree__note">
            The listing stopped early, so some notes are not shown.
          </p>
        ) : null}
      </div>
    </div>
  );
}

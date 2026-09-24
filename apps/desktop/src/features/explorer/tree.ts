/**
 * Turning a flat list of vault paths into the sidebar's tree.
 *
 * Pure, so the shape of the tree — which is what the screenshots are mostly
 * about — can be pinned down without rendering anything.
 */

import { baseName, parentOf, titleOf } from "../../api/source";

export interface TreeNode {
  /** Vault-relative path. A folder has no extension. */
  path: string;
  /** What the row shows: a folder's name, or a note's name without `.md`. */
  label: string;
  kind: "directory" | "document";
  /** Read-only, by its own rule or an enclosing folder's. */
  locked: boolean;
  children: TreeNode[];
}

export interface TreeInput {
  path: string;
  kind: "directory" | "document";
  locked?: boolean;
}

export type SortOrder = "name" | "name-desc" | "modified";

/**
 * Build the tree.
 *
 * The recursive listing reports every folder it reaches, but a folder past
 * its depth limit can still be implied by a note inside it, so folders are also
 * inferred from paths. An inferred folder takes its lock state from its parent,
 * which is the one rule that can reach it without being listed.
 */
export function buildTree(entries: TreeInput[], order: SortOrder = "name"): TreeNode[] {
  const root: TreeNode[] = [];
  const folders = new Map<string, TreeNode>();

  const folderAt = (path: string): TreeNode => {
    const existing = folders.get(path);
    if (existing) return existing;

    const parent = path.includes("/") ? folderAt(parentOf(path)) : null;
    const node: TreeNode = {
      path,
      label: baseName(path),
      kind: "directory",
      locked: explicit.get(path) ?? parent?.locked ?? false,
      children: [],
    };
    folders.set(path, node);

    if (parent) parent.children.push(node);
    else root.push(node);

    return node;
  };

  const explicit = new Map<string, boolean>();
  for (const entry of entries) {
    if (entry.kind === "directory" && entry.path) explicit.set(entry.path, entry.locked ?? false);
  }

  // Folders first, so a directory that also appears implicitly is only made
  // once and keeps the identity the explicit entry gave it.
  for (const entry of entries) {
    if (entry.kind === "directory" && entry.path) folderAt(entry.path);
  }

  for (const entry of entries) {
    if (entry.kind !== "document" || !entry.path) continue;
    const node: TreeNode = {
      path: entry.path,
      label: titleOf(entry.path),
      kind: "document",
      locked: entry.locked ?? false,
      children: [],
    };
    const slash = entry.path.lastIndexOf("/");
    if (slash === -1) root.push(node);
    else folderAt(entry.path.slice(0, slash)).children.push(node);
  }

  sort(root, order);
  return root;
}

/** Folders above files, then by the chosen order — the way a file tree reads. */
function sort(nodes: TreeNode[], order: SortOrder): void {
  nodes.sort((a, b) => {
    if (a.kind !== b.kind) return a.kind === "directory" ? -1 : 1;
    const byName = a.label.localeCompare(b.label, undefined, { numeric: true });
    return order === "name-desc" ? -byName : byName;
  });
  for (const node of nodes) sort(node.children, order);
}

/** Every folder path in the tree, for expanding or collapsing all of it. */
export function folderPaths(nodes: TreeNode[]): string[] {
  return nodes.flatMap((node) =>
    node.kind === "directory" ? [node.path, ...folderPaths(node.children)] : [],
  );
}

/** Whether a node or anything beneath it is locked. */
export function containsLocked(node: TreeNode): boolean {
  return node.locked || node.children.some(containsLocked);
}

/** Every folder path whose own state is locked, for testing drop targets. */
export function lockedFolders(nodes: TreeNode[]): Set<string> {
  const found = new Set<string>();
  const walk = (list: TreeNode[]) => {
    for (const node of list) {
      if (node.kind !== "directory") continue;
      if (node.locked) found.add(node.path);
      walk(node.children);
    }
  };
  walk(nodes);
  return found;
}

/** Every folder that has to be open for `path` to be visible. */
export function ancestorsOf(path: string): string[] {
  const parts = path.split("/");
  parts.pop();
  return parts.map((_, index) => parts.slice(0, index + 1).join("/"));
}

/**
 * Turning a flat list of vault paths into the sidebar's tree.
 *
 * Pure, so the shape of the tree — which is what the screenshots are mostly
 * about — can be pinned down without rendering anything.
 */

import { baseName, isProtected, titleOf } from "../../api/source";

export interface TreeNode {
  /** Vault-relative path. A folder has no extension. */
  path: string;
  /** What the row shows: a folder's name, or a note's name without `.md`. */
  label: string;
  kind: "directory" | "document";
  inAios: boolean;
  children: TreeNode[];
}

export interface TreeInput {
  path: string;
  kind: "directory" | "document";
}

export type SortOrder = "name" | "name-desc" | "modified";

/**
 * Build the tree.
 *
 * Folders are inferred from the paths themselves as well as taken from the
 * input, because the link index reports notes only — the protected tree reaches
 * the sidebar through it, and `aios/memories/extended/x.md` has to imply the two
 * folders above it or the subtree would be flat.
 */
export function buildTree(entries: TreeInput[], order: SortOrder = "name"): TreeNode[] {
  const root: TreeNode[] = [];
  const folders = new Map<string, TreeNode>();

  const folderAt = (path: string): TreeNode => {
    const existing = folders.get(path);
    if (existing) return existing;

    const node: TreeNode = {
      path,
      label: baseName(path),
      kind: "directory",
      inAios: isProtected(path),
      children: [],
    };
    folders.set(path, node);

    const slash = path.lastIndexOf("/");
    if (slash === -1) root.push(node);
    else folderAt(path.slice(0, slash)).children.push(node);

    return node;
  };

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
      inAios: isProtected(entry.path),
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

/** Every folder that has to be open for `path` to be visible. */
export function ancestorsOf(path: string): string[] {
  const parts = path.split("/");
  parts.pop();
  return parts.map((_, index) => parts.slice(0, index + 1).join("/"));
}

/**
 * The vault's link index (SPEC §8, §15).
 *
 * `link-graph` resolves every link in the vault in one pass, so it is what the
 * graph, the backlinks panel, wikilink resolution and the quick switcher are all
 * built from. It returns metadata and link endpoints, never file content.
 */

import { invokeCli } from "./cli";
import { CliFailure } from "./documents";
import type { GraphEdge, GraphNode, LinkGraphData, UnresolvedLink } from "./types";

export interface VaultIndex {
  vault: string;
  nodes: GraphNode[];
  edges: GraphEdge[];
  unresolved: UnresolvedLink[];
  truncated: LinkGraphData["truncated"];
  /** Node index by path, for turning a note into a graph position. */
  indexOf: Map<string, number>;
  /** Outgoing links, by source path. */
  outbound: Map<string, string[]>;
  /**
   * Incoming links, by target path — the backlinks panel.
   *
   * Derived here rather than returned by the CLI: it is exactly the reverse of
   * `edges`, and shipping both would double the payload and let the two drift.
   */
  inbound: Map<string, string[]>;
}

export async function loadVaultIndex(vault: string): Promise<VaultIndex> {
  const response = await invokeCli<LinkGraphData>("link-graph", { vault });
  if (!response.ok || !response.data) {
    throw new CliFailure(
      response.error ?? { code: "INTERNAL_ERROR", message: "the link graph returned nothing" },
      response.stderr,
    );
  }
  return buildIndex(vault, response.data);
}

/** Assemble the derived lookups. Pure, so it can be tested without the bridge. */
export function buildIndex(vault: string, data: LinkGraphData): VaultIndex {
  const indexOf = new Map<string, number>();
  data.nodes.forEach((node, position) => indexOf.set(node.path, position));

  const outbound = new Map<string, string[]>();
  const inbound = new Map<string, string[]>();

  for (const edge of data.edges) {
    const from = data.nodes[edge.from];
    const to = data.nodes[edge.to];
    // An edge naming a node that is not in the response would be a bug in the
    // index, not something to render half of.
    if (!from || !to) continue;

    push(outbound, from.path, to.path);
    push(inbound, to.path, from.path);
  }

  return {
    vault,
    nodes: data.nodes,
    edges: data.edges,
    unresolved: data.unresolved,
    truncated: data.truncated,
    indexOf,
    outbound,
    inbound,
  };
}

function push(map: Map<string, string[]>, key: string, value: string): void {
  const existing = map.get(key);
  if (existing) existing.push(value);
  else map.set(key, [value]);
}

/** Notes that link to `path`, in the index's own order. */
export function backlinksOf(index: VaultIndex | null, path: string): string[] {
  return index?.inbound.get(path) ?? [];
}

/**
 * What a move will carry with it.
 *
 * A rename is followed by `relink`, which retargets the links that pointed at
 * the old path. This is the count the user is told before it happens, and it is
 * free because the index already knows who points where.
 */
export function inboundLinkCount(index: VaultIndex | null, path: string): number {
  return backlinksOf(index, path).length;
}

/**
 * The notes a move will have to rewrite, whether it moves a note or a folder.
 *
 * A folder has no backlinks of its own — nothing links to a directory — so
 * asking `backlinksOf` about one always answers nothing, which for a rename
 * that repaths every note inside is the wrong answer rather than a small one.
 * Everything underneath it is asked about instead.
 *
 * A link from inside the folder to another note inside it is left out: both
 * ends move together, so nothing about it changes, and naming it would pad the
 * list the user is being asked to agree to.
 *
 * Sorted, and each note named once however many times it links.
 */
export function backlinksUnder(index: VaultIndex | null, prefix: string): string[] {
  if (!index) return [];
  const inside = (path: string) => path === prefix || path.startsWith(`${prefix}/`);

  const sources = new Set<string>();
  for (const [target, linking] of index.inbound) {
    if (!inside(target)) continue;
    for (const source of linking) if (!inside(source)) sources.add(source);
  }
  return [...sources].sort();
}

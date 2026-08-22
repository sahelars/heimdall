/**
 * The vault index: one call, three panes (SPEC §8, §15).
 *
 * `link-graph` is the only operation that sees the whole vault, `aios/`
 * included, so it is what the file tree, the graph, and the backlinks panel are
 * all built from. Building them separately would mean the sidebar and the graph
 * could disagree about what the vault contains.
 *
 * It returns metadata and link endpoints, never file content — which is the
 * reason it is allowed to see the protected tree at all.
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
 * What a rename would break.
 *
 * Heimdall deliberately does not rewrite other notes' links when a file moves —
 * that would be an unbounded multi-file write with no revision check on any of
 * it. The client warns instead, which it can do because the index already knows
 * who points where.
 */
export function inboundLinkCount(index: VaultIndex | null, path: string): number {
  return backlinksOf(index, path).length;
}

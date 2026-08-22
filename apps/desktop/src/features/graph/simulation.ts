/**
 * The force layout.
 *
 * These are Obsidian's own forces translated into d3's units. Obsidian's engine
 * is a hand-ported d3-force running in WebAssembly, and the JS fallback it
 * ships spells the stack out literally:
 *
 *     forceLink().distance(250).strength(1 / min(deg_source, deg_target))
 *     forceManyBody().strength(-1000).distanceMin(30).theta(0.9)
 *     forceCollide().radius(60).strength(0.5)
 *     forceX(0).strength(0.1) + forceY(0).strength(0.1)
 *     velocityDecay(0.4), alphaDecay(1 - 0.001^(1/300))
 *
 * Two things about that are worth stating plainly, because both are easy to get
 * wrong and neither is documented:
 *
 * 1. **It is not `forceCenter`.** Obsidian pulls toward a point with `forceX`
 *    and `forceY`. `forceCenter` is a hard, alpha-independent recentring
 *    translation; using it gives a rigid graph that cannot be dragged
 *    off-centre, which is not how Obsidian feels.
 * 2. **Those lengths are device pixels.** Obsidian sizes its canvas in device
 *    pixels, so on a 2x display its 250-unit link distance is 125 CSS px.
 *
 * The numbers below are that layout scaled for a side pane a few hundred CSS
 * pixels wide. The scaling law is that lengths scale by k and charge by k
 * squared, since `forceManyBody` gives a velocity change proportional to Q/L
 * while `forceLink` gives one proportional to strength times L.
 */

import {
  forceCollide,
  forceLink,
  forceManyBody,
  forceSimulation,
  forceX,
  forceY,
  type Simulation,
  type SimulationLinkDatum,
  type SimulationNodeDatum,
} from "d3-force";

export interface GraphSettings {
  /** Pull toward the middle of the pane. Obsidian's "Center force". */
  centerStrength: number;
  /** How hard nodes push apart. Obsidian's "Repel force", as a d3 charge. */
  repelStrength: number;
  /** Multiplier on each link's spring. Obsidian's "Link force". */
  linkStrength: number;
  /** The length a link settles at. Obsidian's "Link distance". */
  linkDistance: number;
  /**
   * Which zoom octave labels turn on at.
   *
   * Obsidian's "Text fade threshold". Labels are invisible below `2^(t-1)` and
   * fully opaque at `2^t`.
   */
  textFadeThreshold: number;
  showOrphans: boolean;
  hideUnresolved: boolean;
}

export const GRAPH_DEFAULTS: GraphSettings = {
  centerStrength: 0.1,
  repelStrength: 60,
  linkStrength: 1,
  linkDistance: 60,
  textFadeThreshold: 0,
  showOrphans: true,
  hideUnresolved: false,
};

/** Obsidian's alpha decay: cools to the 0.001 floor over about 300 ticks. */
export const ALPHA_DECAY = 1 - Math.pow(0.001, 1 / 300);
/** What a drag, a data change, or a force change reheats to. */
export const REHEAT_ALPHA = 0.3;

export interface SimNode extends SimulationNodeDatum {
  id: string;
  title: string;
  inAios: boolean;
  degree: number;
}

export interface SimLink extends SimulationLinkDatum<SimNode> {
  source: string | SimNode;
  target: string | SimNode;
}

export interface GraphModel {
  nodes: SimNode[];
  links: SimLink[];
}

export interface GraphInput {
  nodes: { path: string; title: string; in_aios: boolean }[];
  edges: { from: number; to: number }[];
}

/**
 * Build the model the simulation runs over.
 *
 * `previous` carries positions across a rebuild. The index is refetched after
 * every write, and without this every save would drop the whole layout back to
 * a fresh spiral and re-simulate it — the graph visibly scattering each time
 * someone finishes a sentence.
 */
export function buildModel(
  input: GraphInput,
  settings: GraphSettings,
  previous?: GraphModel | null,
): GraphModel {
  // Obsidian counts a node's weight as outgoing plus incoming links, so a
  // reciprocal pair counts twice for each end.
  const degree = new Map<string, number>();
  for (const edge of input.edges) {
    const from = input.nodes[edge.from];
    const to = input.nodes[edge.to];
    if (!from || !to) continue;
    degree.set(from.path, (degree.get(from.path) ?? 0) + 1);
    degree.set(to.path, (degree.get(to.path) ?? 0) + 1);
  }

  const keep = input.nodes.filter(
    (node) => settings.showOrphans || (degree.get(node.path) ?? 0) > 0,
  );
  const kept = new Set(keep.map((node) => node.path));
  const placed = new Map(previous?.nodes.map((node) => [node.id, node]) ?? []);

  return {
    nodes: keep.map((node) => {
      const before = placed.get(node.path);
      return {
        id: node.path,
        title: node.title,
        inAios: node.in_aios,
        degree: degree.get(node.path) ?? 0,
        // Undefined for a note new to the graph, which is what tells d3 to
        // place it rather than leave it at the origin.
        x: before?.x,
        y: before?.y,
        vx: before?.vx,
        vy: before?.vy,
      };
    }),
    links: input.edges.flatMap((edge) => {
      const from = input.nodes[edge.from];
      const to = input.nodes[edge.to];
      if (!from || !to || !kept.has(from.path) || !kept.has(to.path)) return [];
      return [{ source: from.path, target: to.path }];
    }),
  };
}

export function createSimulation(
  model: GraphModel,
  settings: GraphSettings,
  size: { width: number; height: number },
): Simulation<SimNode, SimLink> {
  const degreeOf = (node: string | SimNode) =>
    Math.max(typeof node === "string" ? 1 : node.degree, 1);

  return forceSimulation(model.nodes)
    .force(
      "link",
      forceLink<SimNode, SimLink>(model.links)
        .id((node) => node.id)
        .distance(settings.linkDistance)
        // d3's own default. Weak springs between well-connected notes, strong
        // ones between notes that have few — which is what stops a hub from
        // dragging its whole neighbourhood into a knot.
        .strength(
          (link) =>
            settings.linkStrength / Math.min(degreeOf(link.source), degreeOf(link.target)),
        ),
    )
    .force(
      "charge",
      forceManyBody<SimNode>()
        .strength(-settings.repelStrength)
        .distanceMin(8)
        // Obsidian has no maximum; this one sits far outside any settled
        // layout, so it costs nothing visually and bounds the work on a large
        // vault.
        .distanceMax(400)
        .theta(0.9),
    )
    // A spacing force rather than a "do not overlap the circle" one: the radius
    // is several times the drawn one, which is what stops a small vault
    // clumping into a single blob.
    .force("collide", forceCollide<SimNode>().radius(15).strength(0.5))
    .force("x", forceX<SimNode>(size.width / 2).strength(settings.centerStrength))
    .force("y", forceY<SimNode>(size.height / 2).strength(settings.centerStrength))
    .velocityDecay(0.4)
    .alphaDecay(ALPHA_DECAY);
}

/**
 * A node's radius in CSS pixels at zoom 1.
 *
 * Obsidian's is `max(8, min(3 * sqrt(degree + 1), 30))` in device pixels; this
 * is that halved. Note the floor: for a vault of a few dozen notes almost every
 * node comes out at the minimum, which is why an Obsidian graph reads as evenly
 * sized dots rather than a spread of sizes.
 */
export function radiusOf(node: SimNode): number {
  return Math.max(4, Math.min(1.5 * Math.sqrt(node.degree + 1), 15));
}

/** Whether two models describe the same set of notes and links. */
export function sameShape(a: GraphModel | null, b: GraphModel | null): boolean {
  if (!a || !b) return a === b;
  if (a.nodes.length !== b.nodes.length || a.links.length !== b.links.length) return false;
  return a.nodes.every((node, index) => node.id === b.nodes[index]?.id);
}

/** Every node one step from `id`, itself included. */
export function neighboursOf(model: GraphModel, id: string): Set<string> {
  const near = new Set<string>([id]);
  for (const link of model.links) {
    const source = typeof link.source === "string" ? link.source : link.source.id;
    const target = typeof link.target === "string" ? link.target : link.target.id;
    if (source === id) near.add(target);
    if (target === id) near.add(source);
  }
  return near;
}

/**
 * How much of the pane a fitted graph fills, along whichever axis is tighter.
 *
 * The old fit reserved a fixed 40px on each side and refused to magnify past
 * 1.4, which for an ordinary vault meant the second limit bound first: a
 * thirteen-note graph came out two thirds of the way across a side pane and a
 * quarter of the way down it, adrift in its own pane.
 */
export const FIT_FILL = 0.88;

/**
 * The narrowest extent a fit will magnify to fill the pane.
 *
 * Two linked notes are sixty units apart. Without a floor, filling the pane
 * with them would draw two dots the size of coins and call it a graph.
 */
const MIN_FIT_SPAN = 120;

/**
 * The most a fit will magnify, whatever the pane.
 *
 * `MIN_FIT_SPAN` already holds a small graph in check at the pane sizes this
 * application has; this is the backstop for a pane dragged very wide.
 */
const MAX_FIT_SCALE = 4;

/**
 * A transform that fits every placed node into `size`, filling `FIT_FILL` of
 * whichever axis is tighter.
 *
 * Without this the graph is drawn from the world origin at the configured
 * scale, which pushes most of it past the bottom-right edge of the pane — the
 * layout is fine and simply not where anyone can see it.
 *
 * The extent measured is the node *positions*. The remaining twelve percent —
 * six a side — is what catches the outermost node's radius and the label under
 * it, which no position knows about.
 */
export function fitToView(
  nodes: readonly SimNode[],
  size: { width: number; height: number },
  maxScale = MAX_FIT_SCALE,
): { x: number; y: number; k: number } {
  const placed = nodes.filter((node) => node.x !== undefined && node.y !== undefined);
  if (placed.length === 0 || size.width === 0 || size.height === 0) {
    return { k: 1, x: 0, y: 0 };
  }

  const xs = placed.map((node) => node.x!);
  const ys = placed.map((node) => node.y!);
  const minX = Math.min(...xs);
  const maxX = Math.max(...xs);
  const minY = Math.min(...ys);
  const maxY = Math.max(...ys);

  const k = Math.max(
    0.1,
    Math.min(
      (size.width * FIT_FILL) / Math.max(maxX - minX, MIN_FIT_SPAN),
      (size.height * FIT_FILL) / Math.max(maxY - minY, MIN_FIT_SPAN),
      maxScale,
    ),
  );

  return {
    k,
    x: size.width / 2 - ((minX + maxX) / 2) * k,
    y: size.height / 2 - ((minY + maxY) / 2) * k,
  };
}

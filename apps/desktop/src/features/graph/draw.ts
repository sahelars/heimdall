/**
 * Painting the graph.
 *
 * Everything is drawn in **screen space**: node positions are projected through
 * the view transform by hand rather than by scaling the canvas. Scaling the
 * canvas is the obvious approach and it costs you control over the two things
 * that matter most here — how big a label is, and how thick a line is — because
 * both then scale with the zoom whether you want them to or not.
 *
 * The device pixel ratio is applied exactly once, as the canvas transform, and
 * every coordinate below is in CSS pixels.
 *
 * A pure function over a context interface, so what it draws can be asserted
 * without a real canvas — jsdom has none, and a mocking library would only
 * prove the canvas API was touched rather than that the right thing was drawn.
 * Colours arrive as values already read from CSS; nothing here knows what any
 * of them are, which is what keeps the palette in the stylesheet.
 */

import { radiusOf, type SimLink, type SimNode } from "./simulation";
import type { Transform } from "./interaction";

export interface GraphColours {
  node: string;
  active: string;
  edge: string;
  label: string;
}

export interface DrawOptions {
  /** Canvas size in CSS pixels. */
  width: number;
  height: number;
  /** `devicePixelRatio`, applied once as the canvas transform. */
  ratio: number;
  transform: Transform;
  activeId: string | null;
  hoverId: string | null;
  /**
   * The hovered note and everything one link from it.
   *
   * Obsidian dims the rest of the graph while you hover, which is what turns a
   * hairball into something you can read a neighbourhood out of. Null means
   * nothing is hovered and everything is drawn at full strength.
   */
  highlighted: ReadonlySet<string> | null;
  /**
   * Per-node dim level, carried between frames and stepped toward its target.
   *
   * Obsidian lerps every alpha 10% of the way per frame — a half-life of about
   * seven frames — so the dimming bleeds in rather than switching. Mutated in
   * place, because it is per-frame animation state rather than data.
   */
  fade: Map<string, number>;
  colours: GraphColours;
  /** A resolved family, never a custom property — see `readFontFamily`. */
  font: string;
  /**
   * Which zoom octave labels turn on at.
   *
   * Obsidian's formula, exactly: `clamp(log2(zoom) + 1 - threshold, 0, 1)`. So
   * at the default of 0, labels are invisible at or below half zoom and fully
   * opaque at or above 1.
   */
  textFadeThreshold: number;
}

/** How faint a node gets while something else is hovered. Obsidian's value. */
const DIMMED = 0.2;

/** How much of the remaining gap a fade closes each frame. Obsidian's value. */
const FADE_RATE = 0.1;

/**
 * Label size in CSS pixels at zoom 1, before the square-root scaling.
 *
 * Labels are drawn at `LABEL_SIZE * sqrt(zoom)`, so this is not the size anyone
 * reads: it is the size at a zoom nobody sits at. What matters is where it
 * lands at the default fit, which now fills 88% of the pane and comes out near
 * 1.9x in a side pane — and there this puts the label at 12px, the same small
 * text the rest of the application is set in. At 11 it grew to 15px as the fit
 * got bigger, which read as a different application's typography.
 */
const LABEL_SIZE = 8.75;

/** Line width in CSS pixels, constant at every zoom — as Obsidian draws it. */
const LINE_WIDTH = 1;

/** Only what this drawing needs, so a test can stand in for it. */
export type Painter = Pick<
  CanvasRenderingContext2D,
  | "save"
  | "restore"
  | "clearRect"
  | "beginPath"
  | "arc"
  | "moveTo"
  | "lineTo"
  | "stroke"
  | "fill"
  | "fillText"
  | "setTransform"
> & {
  fillStyle: string | CanvasGradient | CanvasPattern;
  strokeStyle: string | CanvasGradient | CanvasPattern;
  globalAlpha: number;
  lineWidth: number;
  font: string;
  textAlign: CanvasTextAlign;
  textBaseline: CanvasTextBaseline;
};

export function draw(
  context: Painter,
  model: { nodes: SimNode[]; links: SimLink[] },
  options: DrawOptions,
): void {
  const { transform, colours, highlighted } = options;

  // One transform for the whole frame: device pixels per CSS pixel. Clearing
  // and drawing then share a coordinate system, which is what stops the canvas
  // keeping the previous frame around as a smear.
  context.setTransform(options.ratio, 0, 0, options.ratio, 0, 0);
  context.clearRect(0, 0, options.width, options.height);

  // Obsidian scales its container by the zoom and counter-scales every node and
  // label by 1/sqrt(zoom), so both grow at half the rate of the layout. That
  // half-rate growth is the signature of how its graph feels, and drawing
  // things at full zoom instead is the single easiest way to get it wrong.
  const zoom = Math.sqrt(clamp(transform.k, 1 / 128, 8));

  const screen = (node: SimNode) => ({
    x: node.x! * transform.k + transform.x,
    y: node.y! * transform.k + transform.y,
  });

  // Step each node's dim level toward where it should be.
  for (const node of model.nodes) {
    const target = highlighted === null || highlighted.has(node.id) ? 1 : DIMMED;
    const current = options.fade.get(node.id) ?? target;
    options.fade.set(node.id, current + (target - current) * FADE_RATE);
  }
  const alphaOf = (id: string) => options.fade.get(id) ?? 1;

  // Links first, so a node's disc sits over the lines meeting it. They are
  // trimmed to the circles at both ends, so a line never passes under a node.
  context.lineWidth = LINE_WIDTH;
  for (const link of model.links) {
    const source = link.source as SimNode;
    const target = link.target as SimNode;
    if (!placed(source) || !placed(target)) continue;

    const from = screen(source);
    const to = screen(target);
    const span = Math.hypot(to.x - from.x, to.y - from.y);
    if (span < 1) continue;

    const startRadius = radiusOf(source) * zoom;
    const endRadius = radiusOf(target) * zoom;
    if (span <= startRadius + endRadius) continue;

    const touchesHover =
      options.hoverId !== null &&
      (source.id === options.hoverId || target.id === options.hoverId);

    // A link is lit when it touches what is hovered; everything else recedes.
    context.globalAlpha = highlighted === null || touchesHover ? 1 : DIMMED;
    context.strokeStyle = touchesHover ? colours.active : colours.edge;
    context.beginPath();
    context.moveTo(
      from.x + ((to.x - from.x) * startRadius) / span,
      from.y + ((to.y - from.y) * startRadius) / span,
    );
    context.lineTo(
      to.x - ((to.x - from.x) * endRadius) / span,
      to.y - ((to.y - from.y) * endRadius) / span,
    );
    context.stroke();
  }

  for (const node of model.nodes) {
    if (!placed(node)) continue;
    const at = screen(node);
    const radius = radiusOf(node) * zoom;
    const isActive = node.id === options.activeId || node.id === options.hoverId;

    context.globalAlpha = alphaOf(node.id);
    context.beginPath();
    context.arc(at.x, at.y, radius, 0, Math.PI * 2);
    // A flat filled circle. Obsidian draws no stroke, no glow, no gradient.
    context.fillStyle = isActive ? colours.active : colours.node;
    context.fill();

    // A hairline ring around whatever the pointer is over.
    if (node.id === options.hoverId) {
      context.globalAlpha = 1;
      context.strokeStyle = colours.active;
      context.lineWidth = 1;
      context.beginPath();
      context.arc(at.x, at.y, radius + 1.5, 0, Math.PI * 2);
      context.stroke();
    }
  }

  // Labels fade in over one octave of zoom, which is what Obsidian's "text fade
  // threshold" actually controls.
  const legibility = clamp(
    Math.log2(transform.k) + 1 - options.textFadeThreshold,
    0,
    1,
  );
  if (legibility > 0.001) {
    context.font = `${(LABEL_SIZE * zoom).toFixed(1)}px ${options.font}`;
    context.textAlign = "center";
    context.textBaseline = "top";

    for (const node of model.nodes) {
      if (!placed(node)) continue;
      const at = screen(node);
      // The hovered note's label is always legible, whatever the zoom.
      const visible = node.id === options.hoverId ? 1 : legibility;

      context.globalAlpha = alphaOf(node.id) * visible;
      context.fillStyle = node.id === options.activeId ? colours.active : colours.label;
      context.fillText(node.title, at.x, at.y + radiusOf(node) * zoom + 4);
    }
  }

  // Left as found, so the next frame does not inherit a dimmed alpha.
  context.globalAlpha = 1;
}

function placed(node: SimNode | undefined): boolean {
  return Boolean(node) && node!.x !== undefined && node!.y !== undefined;
}

function clamp(value: number, low: number, high: number): number {
  return Math.min(Math.max(value, low), high);
}

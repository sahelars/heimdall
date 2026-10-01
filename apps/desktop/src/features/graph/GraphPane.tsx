/**
 * The right pane: the vault's link graph on a canvas.
 *
 * Built to match the graph view a reader of Markdown vaults expects. The
 * details that matter, and that are easy to get wrong:
 *
 * - **Node and label size scale with the square root of the zoom**, not with
 *   the zoom. The container scales by the zoom while every node and label
 *   counter-scales by `1/sqrt(zoom)`; that half-rate growth is the signature of
 *   how the graph feels.
 * - **Line thickness is constant on screen** at every zoom.
 * - **Zoom is eased**, closing 15% of the gap per frame, and anchors on the
 *   pointer.
 * - **Panning has inertia**, decaying at 0.9 per frame.
 * - **Everything fades** — hover dimming, colours — by lerping 10% per frame
 *   rather than switching.
 *
 * Drawing happens in screen space (see `draw.ts`), and the device pixel ratio
 * is applied exactly once. Colours come from the stylesheet rather than from
 * here, which is what keeps the whole palette in the file `design.test.ts`
 * reads.
 */

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { Simulation } from "d3-force";

import type { LinkGraphData } from "../../api/types";
import { IconRecentre } from "../../components/icons";
import { draw, type GraphColours } from "./draw";
import {
  approach,
  CLICK_SLOP,
  clampScale,
  ease,
  hitTest,
  PAN_FRICTION,
  screenToWorld,
  wheelFactor,
  ZOOM_EASE,
  type Transform,
} from "./interaction";
import {
  buildModel,
  createSimulation,
  fitToView,
  GRAPH_DEFAULTS,
  neighboursOf,
  radiusOf,
  REHEAT_ALPHA,
  sameShape,
  type GraphModel,
  type GraphSettings,
  type SimNode,
} from "./simulation";

interface GraphPaneProps {
  data: LinkGraphData | null;
  activePath: string | null;
  settings?: GraphSettings;
  onOpen: (path: string) => void;
}

/** How close, in screen pixels, counts as being over a node. */
const HIT_PADDING = 4;

export function GraphPane({ data, activePath, settings = GRAPH_DEFAULTS, onOpen }: GraphPaneProps) {
  const canvas = useRef<HTMLCanvasElement | null>(null);
  const frame = useRef<HTMLDivElement | null>(null);
  const simulation = useRef<Simulation<SimNode, undefined> | null>(null);

  const [size, setSize] = useState({ width: 0, height: 0 });
  const [hover, setHover] = useState<string | null>(null);
  const [built, setBuilt] = useState<GraphModel | null>(null);

  // The view lives in a ref rather than in state: it changes on every frame of
  // a pan or an eased zoom, and re-rendering the tree sixty times a second to
  // move a camera is work nobody asked for.
  const view = useRef<Transform>({ x: 0, y: 0, k: 1 });
  const targetScale = useRef(1);
  /**
   * A transform the view is travelling to, or null when it is standing still.
   *
   * Refitting is not a jump. The layout settles a second or two after a node is
   * dragged, and snapping the camera at that moment reads as the graph skipping
   * — so the fit is flown to over a handful of frames instead.
   */
  const flight = useRef<Transform | null>(null);
  const zoomAnchor = useRef<{ x: number; y: number } | null>(null);
  const panVelocity = useRef({ x: 0, y: 0 });
  /** Whether the user has moved the view themselves. */
  const touched = useRef(false);
  const model = useRef<GraphModel | null>(null);
  const palette = useRef<{ colours: GraphColours; font: string } | null>(null);
  /** Per-node dim level, lerped so hovering fades rather than snaps. */
  const fade = useRef(new Map<string, number>());

  const next = useMemo(
    () => (data ? buildModel({ nodes: data.nodes, edges: data.edges }, settings, model.current) : null),
    [data, settings],
  );

  const highlighted = useMemo(
    () => (hover && built ? neighboursOf(built, hover) : null),
    [hover, built],
  );
  const highlightedRef = useRef(highlighted);
  highlightedRef.current = highlighted;
  const hoverRef = useRef(hover);
  hoverRef.current = hover;
  const activeRef = useRef(activePath);
  activeRef.current = activePath;

  /** Read the palette out of the stylesheet. Expensive; called rarely. */
  const readPalette = useCallback(() => {
    const surface = canvas.current;
    if (!surface) return;
    const style = getComputedStyle(surface);
    const value = (name: string, fallback: string) =>
      style.getPropertyValue(name).trim() || fallback;

    palette.current = {
      colours: {
        node: value("--graph-node", "gray"),
        active: value("--graph-node-active", "gray"),
        edge: value("--graph-edge", "gray"),
        label: value("--graph-label", "gray"),
      },
      // `var(--font)` in a canvas font shorthand is invalid and silently
      // dropped, so the family has to be resolved before it is handed over.
      font: style.fontFamily || "sans-serif",
    };
  }, []);

  /* Sizing ---------------------------------------------------------------- */

  useEffect(() => {
    const element = frame.current;
    if (!element) return;

    const observer = new ResizeObserver(([entry]) => {
      if (!entry) return;
      const width = Math.round(entry.contentRect.width);
      const height = Math.round(entry.contentRect.height);
      // Compared by value: a ResizeObserver fires on any layout change, and a
      // fresh object with identical numbers would restart the simulation.
      setSize((previous) =>
        previous.width === width && previous.height === height ? previous : { width, height },
      );
    });
    observer.observe(element);
    return () => observer.disconnect();
  }, []);

  /* The palette, and the two ways it can change -------------------------- */

  useEffect(() => {
    readPalette();

    const media = window.matchMedia?.("(prefers-color-scheme: dark)");
    const onSystemChange = () => readPalette();
    media?.addEventListener?.("change", onSystemChange);

    // The Settings overrides both land on the root element: the theme as
    // `data-theme`, the accent as an inline custom property — which shows up
    // as a change to `style` and would otherwise leave the graph its old
    // colour until something else made it read the palette again.
    const observer = new MutationObserver(() => readPalette());
    observer.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ["data-theme", "style"],
    });

    return () => {
      media?.removeEventListener?.("change", onSystemChange);
      observer.disconnect();
    };
  }, [readPalette]);

  /* The simulation -------------------------------------------------------- */

  useEffect(() => {
    if (!next || size.width === 0) return;

    // Only rebuild when the set of notes or links actually changed. The index
    // is refetched after every write, and restarting the layout each time would
    // scatter the graph while someone was working.
    const unchanged = sameShape(model.current, next);
    model.current = next;
    setBuilt(next);

    const sim = createSimulation(next, settings, size) as Simulation<SimNode, undefined>;
    simulation.current?.stop();
    simulation.current = sim;

    if (unchanged) {
      sim.alpha(0.15).restart();
      return () => sim.stop();
    }

    // A genuinely different graph. Run it forward off-screen first so the first
    // frame the user sees is a laid-out graph rather than a spiral of nodes
    // unwinding, then frame it.
    sim.stop();
    for (let tick = 0; tick < 240; tick += 1) sim.tick();
    touched.current = false;
    // Instant, this once: there is no previous view to travel from, and the
    // first frame of a new graph should already be framed.
    const fitted = fitToView(next.nodes, size);
    flight.current = null;
    view.current = fitted;
    targetScale.current = fitted.k;
    sim.alpha(REHEAT_ALPHA).restart();

    // Frame it again the moment the layout actually settles. d3 fires "end"
    // when alpha falls under its floor, which is the first point the positions
    // are final — fitting on a timer instead leaves the graph drifting out of
    // frame for the rest of the cooldown.
    //
    // Only while the user has not taken over: refitting under someone who has
    // panned somewhere deliberately would yank the view away from them.
    sim.on("end", () => {
      if (touched.current) return;
      flight.current = fitToView(next.nodes, size);
    });

    return () => {
      sim.on("end", null);
      sim.stop();
    };
  }, [next, settings, size]);

  /* The render loop ------------------------------------------------------- */

  const drag = useRef<{
    x: number;
    y: number;
    lastX: number;
    lastY: number;
    node: SimNode | null;
    moved: boolean;
  } | null>(null);

  useEffect(() => {
    let raf = 0;

    const loop = () => {
      const current = model.current;
      const surface = canvas.current;
      const context = surface?.getContext("2d");

      if (current && surface && context && palette.current && size.width > 0) {
        // A flight owns the whole transform while it lasts, so the eased zoom
        // below does not fight it for the scale.
        if (flight.current) {
          const { view: stepped, done } = approach(view.current, flight.current);
          view.current = stepped;
          targetScale.current = stepped.k;
          if (done) flight.current = null;
        } else if (view.current.k !== targetScale.current) {
          // Eased zoom, anchored so the world point under the pointer stays put.
          const before = view.current.k;
          const after = ease(before, targetScale.current, ZOOM_EASE);
          const anchor = zoomAnchor.current ?? { x: size.width / 2, y: size.height / 2 };
          const applied = after / before;
          view.current = {
            k: after,
            x: anchor.x - (anchor.x - view.current.x) * applied,
            y: anchor.y - (anchor.y - view.current.y) * applied,
          };
        }

        // Pan inertia, which keeps a flick feeling like a flick.
        if (!drag.current && (panVelocity.current.x !== 0 || panVelocity.current.y !== 0)) {
          view.current = {
            ...view.current,
            x: view.current.x + panVelocity.current.x,
            y: view.current.y + panVelocity.current.y,
          };
          panVelocity.current = {
            x: Math.abs(panVelocity.current.x) < 0.05 ? 0 : panVelocity.current.x * PAN_FRICTION,
            y: Math.abs(panVelocity.current.y) < 0.05 ? 0 : panVelocity.current.y * PAN_FRICTION,
          };
        }

        const ratio = window.devicePixelRatio || 1;
        const width = Math.round(size.width * ratio);
        const height = Math.round(size.height * ratio);
        // Assigning either resets the whole context, so only when it changed.
        if (surface.width !== width) surface.width = width;
        if (surface.height !== height) surface.height = height;

        draw(context, current, {
          width: size.width,
          height: size.height,
          ratio,
          transform: view.current,
          activeId: activeRef.current,
          hoverId: hoverRef.current,
          highlighted: highlightedRef.current,
          fade: fade.current,
          colours: palette.current.colours,
          font: palette.current.font,
          textFadeThreshold: settings.textFadeThreshold,
        });
      }

      raf = requestAnimationFrame(loop);
    };

    raf = requestAnimationFrame(loop);
    return () => cancelAnimationFrame(raf);
  }, [size, settings.textFadeThreshold]);

  /* Interaction ----------------------------------------------------------- */

  const pointerAt = (event: React.PointerEvent<HTMLCanvasElement> | React.WheelEvent<HTMLCanvasElement>) => {
    const box = event.currentTarget.getBoundingClientRect();
    return { x: event.clientX - box.left, y: event.clientY - box.top };
  };

  const nodeAt = (screen: { x: number; y: number }) => {
    const current = model.current;
    if (!current) return null;
    const world = screenToWorld(screen, view.current);
    // The hit radius is the drawn radius plus a little, in world units, so it
    // tracks whatever the zoom is doing to the dot.
    const zoom = Math.sqrt(Math.min(Math.max(view.current.k, 0.1), 8));
    return hitTest(current.nodes, world, (12 * zoom + HIT_PADDING) / view.current.k);
  };

  const recentre = useCallback(() => {
    if (!model.current) return;
    flight.current = fitToView(model.current.nodes, size);
    panVelocity.current = { x: 0, y: 0 };
    touched.current = false;
  }, [size]);

  const empty = !built || built.nodes.length === 0;

  return (
    <div className="graph" ref={frame}>
      {empty ? (
        <p className="empty">{data ? "No notes to graph yet." : "Building the graph…"}</p>
      ) : null}

      <canvas
        ref={canvas}
        className="graph__canvas"
        style={{
          width: size.width,
          height: size.height,
          cursor: hover ? "pointer" : undefined,
        }}
        aria-label="Link graph"
        role="img"
        onPointerDown={(event) => {
          if (event.button !== 0) return;
          event.currentTarget.setPointerCapture(event.pointerId);
          panVelocity.current = { x: 0, y: 0 };
          // Whatever the view was travelling to, the user is holding the graph
          // now: a camera that kept moving under a grabbed node would fight it.
          flight.current = null;

          const at = pointerAt(event);
          const node = nodeAt(at);
          if (node) {
            // Pinned where it already is, so the first frame of a drag does not
            // snap it to the cursor, and reheated so its neighbours follow.
            node.fx = node.x;
            node.fy = node.y;
            simulation.current?.alphaTarget(REHEAT_ALPHA).alpha(REHEAT_ALPHA).restart();
          }
          drag.current = {
            x: event.clientX,
            y: event.clientY,
            lastX: event.clientX,
            lastY: event.clientY,
            node,
            moved: false,
          };
        }}
        onPointerMove={(event) => {
          const state = drag.current;

          if (!state) {
            setHover(nodeAt(pointerAt(event))?.id ?? null);
            return;
          }

          if (Math.hypot(event.clientX - state.x, event.clientY - state.y) > CLICK_SLOP) {
            state.moved = true;
          }

          // Deltas tracked here rather than taken from `movementX`, which
          // WebKit reports as zero while a pointer is captured.
          const dx = event.clientX - state.lastX;
          const dy = event.clientY - state.lastY;

          if (state.node) {
            const world = screenToWorld(pointerAt(event), view.current);
            state.node.fx = world.x;
            state.node.fy = world.y;
            simulation.current?.alphaTarget(REHEAT_ALPHA).restart();
          } else {
            touched.current = true;
            view.current = { ...view.current, x: view.current.x + dx, y: view.current.y + dy };
            // Smoothed, so the throw follows the gesture rather than the last
            // single frame of it.
            panVelocity.current = {
              x: panVelocity.current.x * 0.8 + dx * 0.2,
              y: panVelocity.current.y * 0.8 + dy * 0.2,
            };
          }

          state.lastX = event.clientX;
          state.lastY = event.clientY;
        }}
        onPointerUp={(event) => {
          event.currentTarget.releasePointerCapture(event.pointerId);
          const state = drag.current;
          drag.current = null;
          if (!state) return;

          if (state.node) {
            // Released rather than pinned: the node drifts back into the
            // layout instead of staying where it was dropped.
            state.node.fx = null;
            state.node.fy = null;
            simulation.current?.alphaTarget(0);
            panVelocity.current = { x: 0, y: 0 };
          }
          // A press that did not travel is a click, which opens the note.
          if (!state.moved && state.node) onOpen(state.node.id);
        }}
        onPointerCancel={() => {
          drag.current = null;
        }}
        onPointerLeave={() => setHover(null)}
        onWheel={(event) => {
          touched.current = true;
          flight.current = null;
          zoomAnchor.current = pointerAt(event);
          targetScale.current = clampScale(
            targetScale.current * wheelFactor(event.deltaY, event.deltaMode),
          );
        }}
      />

      {empty ? null : (
        <button
          type="button"
          className="graph__recentre"
          title="Recentre"
          aria-label="Recentre"
          onClick={recentre}
        >
          <IconRecentre size={14} />
        </button>
      )}

      {truncationOf(data) ? (
        <p className="graph__truncation" role="status">
          {truncationOf(data)}
        </p>
      ) : null}
    </div>
  );
}

/**
 * What the index had to leave out.
 *
 * A graph that quietly showed most of a vault would be worse than one that said
 * so: every missing node also makes real links look broken.
 */
function truncationOf(data: LinkGraphData | null): string | null {
  const cut = data?.truncated;
  if (!cut) return null;

  const parts: string[] = [];
  if (cut.node_cap_hit) parts.push(`${cut.nodes_omitted} notes not shown`);
  if (cut.total_bytes_cap_hit) parts.push("scan budget reached");
  else if (cut.files_unscanned > 0) parts.push(`${cut.files_unscanned} not scanned for links`);
  return parts.length > 0 ? parts.join(" · ") : null;
}

/** Re-exported so callers can size a node the same way the canvas does. */
export { radiusOf };

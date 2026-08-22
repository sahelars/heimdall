/**
 * What the graph actually paints.
 *
 * The context is a recording stand-in rather than a mocked canvas: asserting on
 * the calls says something about the drawing, where asserting that the canvas
 * API was touched would not.
 */

import { describe, expect, it } from "vitest";

import { draw, type DrawOptions, type Painter } from "./draw";
import type { SimLink, SimNode } from "./simulation";

const COLOURS = { node: "NODE", active: "ACTIVE", edge: "EDGE", label: "LABEL" };

interface Recording {
  painter: Painter;
  arcs: { x: number; y: number; r: number; fill: string; alpha: number }[];
  /** Every `setTransform` call, as its six arguments. */
  transforms: number[][];
  cleared: { x: number; y: number; width: number; height: number }[];
  labels: { text: string; fill: string; font: string; alpha: number }[];
  /** One entry per stroke: the alpha each edge was drawn at. */
  edgeAlphas: number[];
  /** One entry per stroke, so edge thickness can be checked. */
  lineWidths: number[];
}

function recorder(): Recording {
  const arcs: Recording["arcs"] = [];
  const labels: Recording["labels"] = [];
  const lineWidths: number[] = [];
  const edgeAlphas: number[] = [];
  const transforms: number[][] = [];
  const cleared: { x: number; y: number; width: number; height: number }[] = [];
  let pending: { x: number; y: number; r: number } | null = null;

  const painter = {
    fillStyle: "",
    strokeStyle: "",
    globalAlpha: 1,
    lineWidth: 0,
    font: "",
    textAlign: "start" as CanvasTextAlign,
    textBaseline: "alphabetic" as CanvasTextBaseline,
    save() {},
    restore() {},
    clearRect(x: number, y: number, width: number, height: number) {
      cleared.push({ x, y, width, height });
    },
    setTransform(a: number, b: number, c: number, d: number, e: number, f: number) {
      transforms.push([a, b, c, d, e, f]);
    },
    beginPath() {
      pending = null;
    },
    arc(x: number, y: number, r: number) {
      pending = { x, y, r };
    },
    moveTo() {},
    lineTo() {},
    stroke() {
      lineWidths.push(painter.lineWidth);
      edgeAlphas.push(painter.globalAlpha);
    },
    fill() {
      if (pending) arcs.push({ ...pending, fill: String(painter.fillStyle), alpha: painter.globalAlpha });
    },
    fillText(text: string) {
      labels.push({
        text,
        fill: String(painter.fillStyle),
        font: painter.font,
        alpha: painter.globalAlpha,
      });
    },
  } as unknown as Painter & Record<string, unknown>;

  return { painter, arcs, labels, lineWidths, edgeAlphas, transforms, cleared };
}

function model(): { nodes: SimNode[]; links: SimLink[] } {
  const a: SimNode = { id: "a.md", title: "a", inAios: false, degree: 1, x: 0, y: 0 };
  const b: SimNode = { id: "b.md", title: "b", inAios: false, degree: 1, x: 40, y: 0 };
  return { nodes: [a, b], links: [{ source: a, target: b }] };
}

function options(overrides: Partial<DrawOptions> = {}): DrawOptions {
  return {
    width: 400,
    height: 300,
    transform: { x: 0, y: 0, k: 1 },
    activeId: null,
    hoverId: null,
    colours: COLOURS,
    ratio: 1,
    font: "Helvetica, sans-serif",
    highlighted: null,
    fade: new Map(),
    textFadeThreshold: 0,
    ...overrides,
  };
}

describe("drawing the graph", () => {
  it("draws one dot per node and one line per edge", () => {
    const recording = recorder();
    draw(recording.painter, model(), options());

    expect(recording.arcs).toHaveLength(2);
    expect(recording.lineWidths).toHaveLength(1);
    expect(recording.labels.map((label) => label.text)).toEqual(["a", "b"]);
  });

  it("paints only the active node in the accent", () => {
    const recording = recorder();
    draw(recording.painter, model(), options({ activeId: "a.md" }));

    expect(recording.arcs[0]!.fill).toBe("ACTIVE");
    expect(recording.arcs[1]!.fill).toBe("NODE");
  });

  it("highlights whatever the pointer is over", () => {
    const recording = recorder();
    draw(recording.painter, model(), options({ hoverId: "b.md" }));

    expect(recording.arcs[1]!.fill).toBe("ACTIVE");
  });

  it("sizes labels with a font the canvas can actually parse", () => {
    // A `var(--font)` in the shorthand makes the whole declaration invalid, and
    // the canvas drops it without complaint — labels would sit at the default
    // size forever and the zoom scaling would never take effect.
    const recording = recorder();
    draw(recording.painter, model(), options({ transform: { x: 0, y: 0, k: 2 } }));

    expect(recording.labels[0]!.font).not.toContain("var(");
    expect(fontSize(recording.labels[0]!.font)).toBeGreaterThan(0);
  });

  it("keeps links exactly one pixel wide at every zoom", () => {
    // Obsidian's lines are a constant screen thickness however far in or out
    // the graph is zoomed.
    for (const k of [0.2, 1, 4, 8]) {
      const recording = recorder();
      draw(recording.painter, model(), options({ transform: { x: 0, y: 0, k } }));
      expect(recording.lineWidths[0]).toBe(1);
    }
  });

  it("skips a node the simulation has not placed yet", () => {
    const recording = recorder();
    const unplaced: SimNode = { id: "c.md", title: "c", inAios: false, degree: 0 };
    draw(recording.painter, { nodes: [unplaced], links: [] }, options());

    expect(recording.arcs).toHaveLength(0);
    expect(recording.labels).toHaveLength(0);
  });

});

describe("hovering a neighbourhood", () => {
  it("dims everything that is not the hovered note or one of its links", () => {
    // What turns a hairball into something a neighbourhood can be read out of,
    // and what Obsidian does on hover.
    const recording = recorder();
    const graph = model();
    graph.nodes.push({ id: "c.md", title: "c", inAios: false, degree: 0, x: 90, y: 0 });

    draw(
      recording.painter,
      graph,
      options({ hoverId: "a.md", highlighted: new Set(["a.md", "b.md"]) }),
    );

    const [a, b, c] = recording.arcs;
    expect(a!.alpha).toBe(1);
    expect(b!.alpha).toBe(1);
    expect(c!.alpha).toBeLessThan(1);
    // The edge touches the hovered node, so it stays lit.
    expect(recording.edgeAlphas[0]).toBe(1);
  });

  it("dims an edge that does not touch what is hovered", () => {
    const recording = recorder();
    const graph = model();
    graph.nodes.push({ id: "c.md", title: "c", inAios: false, degree: 1, x: 200, y: 0 });
    graph.nodes.push({ id: "d.md", title: "d", inAios: false, degree: 1, x: 260, y: 0 });
    graph.links.push({ source: graph.nodes[2]!, target: graph.nodes[3]! });

    draw(
      recording.painter,
      graph,
      options({ hoverId: "a.md", highlighted: new Set(["a.md", "b.md"]) }),
    );

    // The a–b edge touches the hovered node; the c–d edge does not.
    expect(recording.edgeAlphas[0]).toBe(1);
    expect(recording.edgeAlphas[1]).toBeLessThan(1);
  });

  it("draws everything at full strength when nothing is hovered", () => {
    const recording = recorder();
    draw(recording.painter, model(), options({ highlighted: null }));

    expect(recording.arcs.every((arc) => arc.alpha === 1)).toBe(true);
    expect(recording.edgeAlphas.every((alpha) => alpha === 1)).toBe(true);
  });

  it("leaves the alpha reset so the next frame does not inherit a dimmed one", () => {
    const recording = recorder();
    draw(recording.painter, model(), options({ highlighted: new Set(["a.md"]) }));

    expect(recording.painter.globalAlpha).toBe(1);
  });

  it("fades toward the dimmed level over frames rather than switching", () => {
    // Obsidian lerps every alpha a tenth of the way per frame, so the dimming
    // bleeds in. Snapping to 0.2 in one frame is a different, harsher thing.
    const fade = new Map<string, number>();
    const graph = model();

    // One frame with nothing hovered, so the nodes start at full strength. A
    // node seen for the first time is initialised to wherever it belongs rather
    // than fading in from nothing.
    draw(recorder().painter, graph, options({ fade }));

    const dim = () => {
      const recording = recorder();
      draw(
        recording.painter,
        graph,
        options({ hoverId: "a.md", highlighted: new Set(["a.md"]), fade }),
      );
      return recording.arcs[1]!.alpha;
    };

    const first = dim();
    const second = dim();
    const later = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0].reduce(dim, 0);

    expect(first).toBeLessThan(1);
    expect(second).toBeLessThan(first);
    expect(later).toBeLessThan(second);
    expect(later).toBeGreaterThan(0.19);
  });
});

describe("zooming", () => {
  it("makes labels bigger as you zoom in", () => {
    // The whole point of zooming in on a graph is to read it. Sizing the font
    // in world units and letting the canvas transform scale it back down keeps
    // labels a constant screen size, which is what this replaced.
    const near = recorder();
    draw(near.painter, model(), options({ transform: { x: 0, y: 0, k: 2 } }));

    const far = recorder();
    draw(far.painter, model(), options({ transform: { x: 0, y: 0, k: 1 } }));

    expect(fontSize(near.labels[0]!.font)).toBeGreaterThan(fontSize(far.labels[0]!.font));
  });

  it("makes nodes bigger as you zoom in", () => {
    const near = recorder();
    draw(near.painter, model(), options({ transform: { x: 0, y: 0, k: 2 } }));

    const far = recorder();
    draw(far.painter, model(), options({ transform: { x: 0, y: 0, k: 1 } }));

    expect(near.arcs[0]!.r).toBeGreaterThan(far.arcs[0]!.r);
  });

  it("drops labels below the zoom the threshold names", () => {
    // Obsidian's formula: labels are invisible at or below 2^(t-1), which at
    // the default threshold of 0 is half zoom.
    const recording = recorder();
    draw(recording.painter, model(), options({ transform: { x: 0, y: 0, k: 0.4 } }));

    expect(recording.arcs).toHaveLength(2);
    expect(recording.labels).toHaveLength(0);
  });

  it("fades labels in across one octave of zoom", () => {
    const recording = recorder();
    draw(recording.painter, model(), options({ transform: { x: 0, y: 0, k: 0.7 } }));

    const alpha = recording.labels[0]!.alpha;
    expect(alpha).toBeGreaterThan(0);
    expect(alpha).toBeLessThan(1);
  });

  it("moves that fade band when the threshold changes", () => {
    const raised = recorder();
    draw(
      raised.painter,
      model(),
      options({ transform: { x: 0, y: 0, k: 1 }, textFadeThreshold: 2 }),
    );
    // At threshold 2 nothing shows until zoom 2, so zoom 1 is blank.
    expect(raised.labels).toHaveLength(0);
  });
});

describe("clearing", () => {
  it("wipes the whole canvas in the same units it draws in", () => {
    // The bug this exists to stop: the clear ran under the identity transform
    // while drawing ran under the device-pixel-ratio one, so on a 2x display
    // only the top-left quarter was ever wiped and every frame smeared over the
    // last.
    const recording = recorder();
    draw(recording.painter, model(), options({ ratio: 2, width: 400, height: 300 }));

    expect(recording.transforms[0]).toEqual([2, 0, 0, 2, 0, 0]);
    expect(recording.cleared).toEqual([{ x: 0, y: 0, width: 400, height: 300 }]);
  });

  it("applies the pixel ratio exactly once", () => {
    const recording = recorder();
    draw(recording.painter, model(), options({ ratio: 2 }));

    // A second transform would scale the graph on top of the ratio, which is
    // what drew everything at half size on a retina display.
    expect(recording.transforms).toHaveLength(1);
  });
});

/** The pixel size out of a canvas font shorthand. */
function fontSize(font: string): number {
  return Number.parseFloat(font);
}

import { describe, expect, it } from "vitest";

import {
  approach,
  clampScale,
  hitTest,
  MAX_SCALE,
  MIN_SCALE,
  pan,
  screenToWorld,
  worldToScreen,
  zoomAbout,
  type Transform,
} from "./interaction";
import {
  buildModel,
  FIT_FILL,
  fitToView,
  GRAPH_DEFAULTS,
  neighboursOf,
  sameShape,
} from "./simulation";

const identity: Transform = { x: 0, y: 0, k: 1 };
const shifted: Transform = { x: 120, y: -40, k: 1.53 };

describe("coordinates", () => {
  it("round-trips between world and screen", () => {
    const point = { x: 37, y: -12 };
    const back = screenToWorld(worldToScreen(point, shifted), shifted);

    expect(back.x).toBeCloseTo(point.x, 10);
    expect(back.y).toBeCloseTo(point.y, 10);
  });

  it("moves by exactly what it is panned", () => {
    expect(pan(identity, 10, -5)).toEqual({ x: 10, y: -5, k: 1 });
  });
});

describe("flying the view to a transform", () => {
  const target: Transform = { x: 200, y: -60, k: 2 };

  it("moves part of the way rather than arriving at once", () => {
    // The whole point: refitting when the layout settles used to assign the
    // transform outright, which read as the graph skipping.
    const { view, done } = approach(identity, target);

    expect(done).toBe(false);
    expect(view.x).toBeGreaterThan(identity.x);
    expect(view.x).toBeLessThan(target.x);
    expect(view.k).toBeGreaterThan(identity.k);
    expect(view.k).toBeLessThan(target.k);
  });

  it("closes the gap monotonically and arrives", () => {
    let current = identity;
    let done = false;
    let frames = 0;

    while (!done && frames < 600) {
      const step = approach(current, target);
      expect(Math.abs(target.x - step.view.x)).toBeLessThanOrEqual(Math.abs(target.x - current.x));
      current = step.view;
      done = step.done;
      frames += 1;
    }

    expect(done).toBe(true);
    expect(current).toEqual(target);
    // Under a second at sixty frames: any slower and it reads as drifting.
    expect(frames).toBeLessThan(60);
  });

  it("arrives at a target of exactly zero", () => {
    // `ease` tests convergence relative to the target, so a centred x or y —
    // which a fitted view produces routinely — would never be reported done and
    // the flight would never end.
    let current: Transform = { x: 40, y: 40, k: 1 };
    let done = false;

    for (let frame = 0; frame < 600 && !done; frame += 1) {
      const step = approach(current, { x: 0, y: 0, k: 1 });
      current = step.view;
      done = step.done;
    }

    expect(done).toBe(true);
    expect(current).toEqual({ x: 0, y: 0, k: 1 });
  });
});

describe("zooming", () => {
  it("keeps the point under the pointer where it was", () => {
    // The property that makes wheel-zoom feel like zooming rather than drifting.
    const pointer = { x: 300, y: 200 };
    const before = screenToWorld(pointer, shifted);
    const after = screenToWorld(pointer, zoomAbout(shifted, pointer, 1.4));

    expect(after.x).toBeCloseTo(before.x, 8);
    expect(after.y).toBeCloseTo(before.y, 8);
  });

  it("stops at the limits rather than inverting or vanishing", () => {
    expect(clampScale(0)).toBe(MIN_SCALE);
    expect(clampScale(1000)).toBe(MAX_SCALE);
    expect(zoomAbout({ ...identity, k: MAX_SCALE }, { x: 0, y: 0 }, 4).k).toBe(MAX_SCALE);
    expect(zoomAbout({ ...identity, k: MIN_SCALE }, { x: 0, y: 0 }, 0.1).k).toBe(MIN_SCALE);
  });
});

describe("hit testing", () => {
  const nodes = [
    { id: "a", x: 0, y: 0 },
    { id: "b", x: 50, y: 0 },
  ];

  it("finds the node under the point", () => {
    expect(hitTest(nodes, { x: 2, y: 2 }, 10)?.id).toBe("a");
    expect(hitTest(nodes, { x: 48, y: 1 }, 10)?.id).toBe("b");
  });

  it("finds nothing in empty space", () => {
    expect(hitTest(nodes, { x: 25, y: 25 }, 5)).toBeNull();
  });

  it("ignores a node the simulation has not placed yet", () => {
    expect(hitTest([{ id: "c" }], { x: 0, y: 0 }, 10)).toBeNull();
  });

  it("picks the nearest when two overlap", () => {
    const close = [
      { id: "far", x: 8, y: 0 },
      { id: "near", x: 1, y: 0 },
    ];
    expect(hitTest(close, { x: 0, y: 0 }, 20)?.id).toBe("near");
  });
});

describe("framing the graph", () => {
  const size = { width: 340, height: 300 };

  it("puts the whole graph inside the viewport", () => {
    // The bug this exists to stop: drawing at the configured scale about the
    // origin left 9 of 13 nodes past the bottom-right edge of the pane.
    const nodes = [
      { id: "a", title: "a", inAios: false, degree: 1, x: 32, y: -12 },
      { id: "b", title: "b", inAios: false, degree: 1, x: 300, y: 317 },
      { id: "c", title: "c", inAios: false, degree: 1, x: 150, y: 150 },
    ];

    const transform = fitToView(nodes, size);

    for (const node of nodes) {
      const screen = worldToScreen({ x: node.x, y: node.y }, transform);
      expect(screen.x).toBeGreaterThanOrEqual(0);
      expect(screen.x).toBeLessThanOrEqual(size.width);
      expect(screen.y).toBeGreaterThanOrEqual(0);
      expect(screen.y).toBeLessThanOrEqual(size.height);
    }
  });

  it("centres the graph rather than pinning it to a corner", () => {
    const nodes = [
      { id: "a", title: "a", inAios: false, degree: 1, x: 0, y: 0 },
      { id: "b", title: "b", inAios: false, degree: 1, x: 100, y: 100 },
    ];

    const middle = worldToScreen({ x: 50, y: 50 }, fitToView(nodes, size));
    expect(middle.x).toBeCloseTo(size.width / 2, 6);
    expect(middle.y).toBeCloseTo(size.height / 2, 6);
  });

  it("fills the pane along whichever axis is tighter", () => {
    // The graph used to come out two thirds of the way across a side pane and a
    // quarter of the way down it: a fixed 40px of padding, and a magnification
    // cap of 1.4 that bound first for any ordinary vault.
    const nodes = [
      { id: "a", title: "a", inAios: false, degree: 1, x: 0, y: 0 },
      { id: "b", title: "b", inAios: false, degree: 1, x: 300, y: 200 },
    ];
    const pane = { width: 400, height: 900 };

    const { k } = fitToView(nodes, pane);
    // Width is the tighter axis here, so width is the one that fills.
    expect((300 * k) / pane.width).toBeCloseTo(FIT_FILL, 6);
    expect((200 * k) / pane.height).toBeLessThan(FIT_FILL);
  });

  it("does not blow a two-note graph up to fill the pane", () => {
    // Two linked notes are sixty units apart. Filled to the pane they would be
    // two dots the size of coins.
    const nodes = [
      { id: "a", title: "a", inAios: false, degree: 1, x: 0, y: 0 },
      { id: "b", title: "b", inAios: false, degree: 1, x: 60, y: 0 },
    ];

    const { k } = fitToView(nodes, { width: 400, height: 900 });
    expect((60 * k) / 400).toBeLessThan(FIT_FILL);
  });

  it("never magnifies past the cap it is given", () => {
    const nodes = [{ id: "a", title: "a", inAios: false, degree: 1, x: 10, y: 10 }];
    expect(fitToView(nodes, size, 1.4).k).toBeLessThanOrEqual(1.4);
  });

  it("falls back to an unscaled view for an empty or unmeasured graph", () => {
    expect(fitToView([], size).k).toBe(1);
    expect(() => fitToView([], { width: 0, height: 0 })).not.toThrow();
  });
});

describe("keeping a layout across a rebuild", () => {
  it("carries node positions over, so a save does not scatter the graph", () => {
    // The index is refetched after every write. Without this the whole layout
    // dropped back to a fresh spiral each time someone finished a sentence.
    const input = {
      nodes: [
        { path: "a.md", title: "a", in_aios: false },
        { path: "b.md", title: "b", in_aios: false },
      ],
      edges: [{ from: 0, to: 1 }],
    };

    const first = buildModel(input, GRAPH_DEFAULTS);
    first.nodes[0]!.x = 123;
    first.nodes[0]!.y = 456;

    const second = buildModel(input, GRAPH_DEFAULTS, first);
    expect(second.nodes[0]!.x).toBe(123);
    expect(second.nodes[0]!.y).toBe(456);
  });

  it("leaves a note that is new to the graph unplaced, so d3 positions it", () => {
    const before = buildModel(
      { nodes: [{ path: "a.md", title: "a", in_aios: false }], edges: [] },
      GRAPH_DEFAULTS,
    );
    before.nodes[0]!.x = 10;

    const after = buildModel(
      {
        nodes: [
          { path: "a.md", title: "a", in_aios: false },
          { path: "new.md", title: "new", in_aios: false },
        ],
        edges: [],
      },
      GRAPH_DEFAULTS,
      before,
    );

    expect(after.nodes[0]!.x).toBe(10);
    expect(after.nodes[1]!.x).toBeUndefined();
  });

  it("recognises the same graph arriving as new objects", () => {
    const input = { nodes: [{ path: "a.md", title: "a", in_aios: false }], edges: [] };
    expect(sameShape(buildModel(input, GRAPH_DEFAULTS), buildModel(input, GRAPH_DEFAULTS))).toBe(true);

    const grown = {
      nodes: [...input.nodes, { path: "b.md", title: "b", in_aios: false }],
      edges: [],
    };
    expect(sameShape(buildModel(input, GRAPH_DEFAULTS), buildModel(grown, GRAPH_DEFAULTS))).toBe(false);
  });
});

describe("neighbourhoods", () => {
  it("names the hovered note and everything one link from it", () => {
    const model = buildModel(
      {
        nodes: [
          { path: "a.md", title: "a", in_aios: false },
          { path: "b.md", title: "b", in_aios: false },
          { path: "c.md", title: "c", in_aios: false },
        ],
        edges: [{ from: 0, to: 1 }],
      },
      GRAPH_DEFAULTS,
    );

    expect([...neighboursOf(model, "a.md")].sort()).toEqual(["a.md", "b.md"]);
    expect([...neighboursOf(model, "c.md")]).toEqual(["c.md"]);
  });
});

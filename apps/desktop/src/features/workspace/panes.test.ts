import { describe, expect, it } from "vitest";

import {
  clampPane,
  COLLAPSE_BELOW,
  defaultPanes,
  DIVIDERS,
  fitPanes,
  MIN_NOTE,
  MIN_PANE,
  paneTrack,
} from "./panes";

describe("sizing a pane", () => {
  it("keeps a pane usable rather than letting it shrink to a sliver", () => {
    expect(clampPane(MIN_PANE + 40, 1200)).toBe(MIN_PANE + 40);
    expect(clampPane(MIN_PANE - 1, 1200)).toBe(MIN_PANE);
  });

  it("reads a drag past the collapse point as putting the pane away", () => {
    // The alternative — snapping back to the minimum — makes the pane
    // impossible to close by dragging, which is how people expect to close it.
    expect(clampPane(COLLAPSE_BELOW - 1, 1200)).toBe(0);
    expect(clampPane(0, 1200)).toBe(0);
  });

  it("lets a pane grow only until the note reaches its floor", () => {
    // The bound is the layout, not a share of the window: 1000 wide, 200 taken
    // by the pane on the other side and 14 by the divider tracks leaves the
    // note MIN_NOTE only if this pane stops at 546.
    expect(clampPane(900, 1000, 200)).toBe(1000 - DIVIDERS - 200 - MIN_NOTE);
    expect(clampPane(400, 1000, 200)).toBe(400);
  });

  it("lets a divider that starts at 40% of the window still travel outward", () => {
    // The old share cap put the graph divider at its own maximum on first run,
    // so the one pane the default made narrow could not be widened at all.
    const window_ = 1440;
    const { left, right } = defaultPanes(window_);
    expect(clampPane(right + 160, window_, left)).toBe(right + 160);
  });

  it("keeps a pane at its minimum in a window too small for the note's floor", () => {
    // Nothing fits; refusing to return a pane at all would hand the grid a
    // negative track, and fitPanes is what closes a pane when it must.
    expect(clampPane(300, 400, 300)).toBe(MIN_PANE);
  });

  it("survives a container that has not been measured yet", () => {
    // jsdom and the first paint both report zero, and clamping to nothing then
    // would collapse both panes on load.
    expect(clampPane(300, 0)).toBe(300);
    expect(clampPane(900, 0, 400)).toBe(900);
  });

  it("spells a collapsed pane as a zero track", () => {
    expect(paneTrack(0)).toBe("0px");
    expect(paneTrack(260)).toBe("260px");
  });
});

describe("fitting the panes to the window", () => {
  it("leaves them alone when the note already has room", () => {
    const panes = { left: 240, right: 280 };
    expect(fitPanes(panes, 1400)).toBe(panes);
  });

  it("takes from the side panes rather than squeezing the note to nothing", () => {
    // The centre track is `1fr`, so without this the two fixed side panes keep
    // every pixel and the note shrinks to a sliver.
    const fitted = fitPanes({ left: 400, right: 400 }, 900);
    const note = 900 - DIVIDERS - fitted.left - fitted.right;

    expect(note).toBeGreaterThanOrEqual(MIN_NOTE);
    expect(fitted.left).toBeLessThan(400);
  });

  it("takes from whichever pane is wider first", () => {
    const fitted = fitPanes({ left: 500, right: 200 }, 900);
    expect(fitted.right).toBe(200);
    expect(fitted.left).toBeLessThan(500);
  });

  it("closes a pane entirely rather than overflowing the window", () => {
    const fitted = fitPanes({ left: 300, right: 300 }, 500);
    expect(fitted.left + fitted.right).toBeLessThanOrEqual(500);
  });

  it("does nothing before the window has been measured", () => {
    const panes = { left: 240, right: 280 };
    expect(fitPanes(panes, 0)).toBe(panes);
  });

  it("keeps the note the widest pane in a window that opens maximized", () => {
    // The window opens filling the display (SPEC §15), so the width to reason
    // about is a real screen's, not the 1440 the configuration nominally asks
    // for. The note is the reason the window is open at either size.
    for (const width of [1440, 1920, 2560, 3440]) {
      const panes = defaultPanes(width);
      const note = width - DIVIDERS - panes.left - panes.right;
      expect(note, `${width}`).toBeGreaterThan(panes.left);
      expect(note, `${width}`).toBeGreaterThan(panes.right);
      expect(fitPanes(panes, width), `${width}`).toBe(panes);
    }
  });
});

describe("the widths the panes open at", () => {
  it("splits the window 14 / 46 / 40 rather than at fixed pixels", () => {
    const width = 1920;
    const { left, right } = defaultPanes(width);
    // The note takes what is left, so it is the 46% less the divider tracks —
    // the two side panes are the shares that are actually asked for.
    const note = width - DIVIDERS - left - right;

    expect(left).toBe(269);
    expect(right).toBe(768);
    expect(note).toBe(869);

    expect(left / width).toBeCloseTo(0.14, 3);
    expect(right / width).toBeCloseTo(0.4, 3);
    // Exactly the remaining share less the divider tracks, which are real
    // pixels the note does not get.
    expect(note).toBe(Math.round(width * 0.46) - DIVIDERS);
    expect(note).toBeGreaterThan(right);
  });

  it("gives whole pixels, since they become grid tracks", () => {
    const { left, right } = defaultPanes(1367);
    expect(Number.isInteger(left)).toBe(true);
    expect(Number.isInteger(right)).toBe(true);
  });

  it("falls back to a sane pair before the window has been measured", () => {
    // Proportions of nothing are nothing, and two collapsed panes on first
    // paint is worse than the fixed pair this replaced.
    const panes = defaultPanes(0);
    expect(panes.left).toBeGreaterThanOrEqual(MIN_PANE);
    expect(panes.right).toBeGreaterThanOrEqual(MIN_PANE);
  });
});

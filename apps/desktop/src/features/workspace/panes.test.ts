import { describe, expect, it } from "vitest";

import {
  clampPane,
  COLLAPSE_BELOW,
  DEFAULT_PANES,
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

  it("never lets one side take most of the window", () => {
    expect(clampPane(900, 1000)).toBe(400);
  });

  it("survives a container that has not been measured yet", () => {
    // jsdom and the first paint both report zero, and clamping to nothing then
    // would collapse both panes on load.
    expect(clampPane(300, 0)).toBe(300);
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
    const note = 900 - 2 - fitted.left - fitted.right;

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

  it("keeps the note the widest pane at the default window size", () => {
    // 880 is the window's configured default; the note is the reason it is open.
    const note = 880 - 2 - DEFAULT_PANES.left - DEFAULT_PANES.right;
    expect(note).toBeGreaterThan(DEFAULT_PANES.left);
    expect(note).toBeGreaterThan(DEFAULT_PANES.right);
  });
});

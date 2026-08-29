/**
 * Pane geometry.
 *
 * Pure, because jsdom reports every element as zero-sized: the drag handler can
 * only be tested for calling this, so this is where the behaviour has to live.
 */

/**
 * The narrowest a side pane is allowed to get.
 *
 * The file pane's toolbar is what sets this: it now starts to the right of the
 * window's own buttons, so it needs the 76px they take, four 28px icon buttons
 * with 2px between them, and the 6px it keeps at its other end — 200 in all.
 * Below that its last icon would be cut off by the divider.
 */
export const MIN_PANE = 204;
/**
 * The narrowest the note is allowed to get.
 *
 * The centre track is `1fr`, so without a floor the two fixed side panes can
 * squeeze it to nothing — the note is the reason the window is open. Below this
 * the note's own content stops fitting and `.note__body` scrolls sideways,
 * which is a better answer than refusing to let the divider travel.
 */
export const MIN_NOTE = 240;
/** Below this, a drag is read as "put it away" rather than "make it tiny". */
export const COLLAPSE_BELOW = 120;
/**
 * The width the two divider tracks take out of the grid.
 *
 * `.workspace` in styles.css spends 7px on each, so this is 14 — the arithmetic
 * here has to agree with the stylesheet or the note's floor is off by a track.
 */
export const DIVIDERS = 14;

export interface PaneWidths {
  left: number;
  right: number;
}

/**
 * Starting proportions: files 14%, note 46%, graph 40%.
 *
 * Shares rather than pixels because the window opens maximized (§15), so the
 * width at launch is the display's — a fixed 240/280 pair that read as
 * reasonable at 880 leaves the graph a sliver on a wide screen.
 */
export const DEFAULT_SHARES = { left: 0.14, right: 0.4 };

/** The pair the shares fall back to before anything has been measured. */
const UNMEASURED_PANES: PaneWidths = { left: 240, right: 280 };

/**
 * Starting widths for a window of this width.
 *
 * Floored at the same minimum a drag is: a share of a small window can be
 * narrower than a pane is allowed to be, and a file pane that opens too narrow
 * to hold its own toolbar has cut an icon off before anyone has touched a
 * divider. `fitPanes` is what gives the note its room back if the floors do not
 * fit; that is its job, and it already runs on every resize.
 */
export function defaultPanes(containerWidth: number): PaneWidths {
  if (containerWidth <= 0) return UNMEASURED_PANES;
  return {
    left: Math.max(MIN_PANE, Math.round(containerWidth * DEFAULT_SHARES.left)),
    right: Math.max(MIN_PANE, Math.round(containerWidth * DEFAULT_SHARES.right)),
  };
}

export function isPaneWidths(value: unknown): value is PaneWidths {
  if (typeof value !== "object" || value === null) return false;
  const candidate = value as Record<string, unknown>;
  return typeof candidate.left === "number" && typeof candidate.right === "number";
}

/**
 * Where a divider should settle.
 *
 * The maximum comes from the layout rather than from a share of the window: a
 * pane may grow until the note reaches its floor, given the divider tracks and
 * whatever the pane on the other side is currently taking. A fixed share capped
 * every divider at the same fraction, which meant a pane opening at that
 * fraction could not be dragged outward at all.
 *
 * A container of zero — which is what jsdom and the first paint both report —
 * has no meaningful maximum, so the requested width passes through the minimum
 * check only rather than being clamped to nothing.
 */
export function clampPane(requested: number, containerWidth: number, opposite = 0): number {
  if (requested < COLLAPSE_BELOW) return 0;
  const room = containerWidth - DIVIDERS - Math.max(opposite, 0) - MIN_NOTE;
  // Never below MIN_PANE: in a window too small to honour the note's floor the
  // pane is already as small as a pane can be, and fitPanes is what decides
  // whether it closes.
  const maximum = containerWidth > 0 ? Math.max(room, MIN_PANE) : Number.POSITIVE_INFINITY;
  return Math.round(Math.min(Math.max(requested, MIN_PANE), maximum));
}

/** The CSS width for a pane, where zero means collapsed. */
export function paneTrack(width: number): string {
  return width <= 0 ? "0px" : `${width}px`;
}

/**
 * Shrink the side panes until the note has room again.
 *
 * Applied on every window resize, not only while dragging: two panes dragged
 * wide in a large window would otherwise overflow the grid when the window
 * shrinks, giving the whole application a horizontal scrollbar and pushing the
 * graph off the right-hand edge.
 */
export function fitPanes(panes: PaneWidths, containerWidth: number): PaneWidths {
  if (containerWidth <= 0) return panes;

  let { left, right } = panes;
  let spare = containerWidth - DIVIDERS - left - right - MIN_NOTE;
  if (spare >= 0) return panes;

  // Take from whichever side is wider, so one huge pane gives ground first.
  while (spare < 0) {
    const takeFromLeft = left >= right;
    const target = takeFromLeft ? left : right;
    if (target <= 0) break;

    const floor = target > MIN_PANE ? MIN_PANE : 0;
    const reduced = Math.max(floor, target + spare);
    const given = target - reduced;
    if (given <= 0) break;

    if (takeFromLeft) left = reduced;
    else right = reduced;
    spare += given;
  }

  return { left, right };
}

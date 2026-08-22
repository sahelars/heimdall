/**
 * Pane geometry.
 *
 * Pure, because jsdom reports every element as zero-sized: the drag handler can
 * only be tested for calling this, so this is where the behaviour has to live.
 */

export const MIN_PANE = 180;
/**
 * The narrowest the note is allowed to get.
 *
 * The centre track is `1fr`, so without a floor the two fixed side panes can
 * squeeze it to nothing — the note is the reason the window is open.
 */
export const MIN_NOTE = 360;
/** Below this, a drag is read as "put it away" rather than "make it tiny". */
export const COLLAPSE_BELOW = 120;
/** No single side may take more than this share of the window. */
export const MAX_SHARE = 0.4;

export interface PaneWidths {
  left: number;
  right: number;
}

/**
 * Starting widths.
 *
 * Chosen against the 880px default window: 240 + 280 leaves 358 for the note,
 * which keeps it the widest of the three on first run.
 */
export const DEFAULT_PANES: PaneWidths = { left: 240, right: 280 };

export function isPaneWidths(value: unknown): value is PaneWidths {
  if (typeof value !== "object" || value === null) return false;
  const candidate = value as Record<string, unknown>;
  return typeof candidate.left === "number" && typeof candidate.right === "number";
}

/**
 * Where a divider should settle.
 *
 * A container of zero — which is what jsdom and the first paint both report —
 * has no meaningful maximum, so the requested width passes through the minimum
 * check only rather than being clamped to nothing.
 */
export function clampPane(requested: number, containerWidth: number): number {
  if (requested < COLLAPSE_BELOW) return 0;
  const maximum = containerWidth > 0 ? containerWidth * MAX_SHARE : Number.POSITIVE_INFINITY;
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

  const dividers = 2;
  let { left, right } = panes;
  let spare = containerWidth - dividers - left - right - MIN_NOTE;
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

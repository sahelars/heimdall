/**
 * Pan, zoom, and hit testing.
 *
 * Pure maths, kept out of the component because jsdom reports every element as
 * zero-sized: a component test could not tell whether any of this were right.
 */

export interface Transform {
  x: number;
  y: number;
  k: number;
}

/** Obsidian's zoom limits. */
export const MIN_SCALE = 1 / 128;
export const MAX_SCALE = 8;

/**
 * How far one wheel notch zooms.
 *
 * Obsidian multiplies the target scale by `1.5^(-deltaY/120)`, so a standard
 * notch is exactly 1.5x.
 */
export function wheelFactor(deltaY: number, deltaMode = 0): number {
  const lines = deltaMode === 1 ? deltaY * 40 : deltaMode === 2 ? deltaY * 800 : deltaY;
  return Math.pow(1.5, -lines / 120);
}

/**
 * How much of the remaining gap a frame closes.
 *
 * Zoom is eased rather than applied outright, which is most of why Obsidian's
 * graph feels smooth: `scale = scale*0.85 + target*0.15` per frame.
 */
export const ZOOM_EASE = 0.15;

/** Pan velocity retained per frame after release. */
export const PAN_FRICTION = 0.9;

/** Move at least this far and a press is a drag rather than a click. */
export const CLICK_SLOP = 5;

/** Step an eased value toward its target. Returns the target once close. */
export function ease(current: number, target: number, rate: number): number {
  const next = current + (target - current) * rate;
  return Math.abs(target - next) < Math.abs(target) * 0.001 ? target : next;
}

/**
 * How much of the remaining gap a frame of a flight closes.
 *
 * A flight is the view travelling to a transform it was given rather than one
 * the user is producing — refitting when the layout settles, or the Recentre
 * button. Slower than the zoom ease, because the whole view moves and a jump
 * that takes three frames still reads as a jump.
 */
export const FLIGHT_EASE = 0.12;

/**
 * Step the view toward a target transform.
 *
 * Reported as done rather than compared by the caller: the thresholds are what
 * decide whether the last fraction of a pixel is worth another frame, and they
 * belong next to the easing that produces it. `ease` cannot be used for this —
 * its convergence test is relative to the target, so a target of exactly zero,
 * which a centred `x` or `y` reaches routinely, never arrives.
 */
export function approach(
  current: Transform,
  target: Transform,
  rate = FLIGHT_EASE,
): { view: Transform; done: boolean } {
  const step = (from: number, to: number) => from + (to - from) * rate;
  const view = {
    x: step(current.x, target.x),
    y: step(current.y, target.y),
    k: step(current.k, target.k),
  };

  // Half a pixel of pan and a fifth of a percent of zoom: below either, the
  // next frame would draw the same picture.
  const done =
    Math.abs(target.x - view.x) < 0.5 &&
    Math.abs(target.y - view.y) < 0.5 &&
    Math.abs(target.k - view.k) < target.k * 0.002;

  return { view: done ? { ...target } : view, done };
}

export function worldToScreen(point: { x: number; y: number }, transform: Transform) {
  return { x: point.x * transform.k + transform.x, y: point.y * transform.k + transform.y };
}

export function screenToWorld(point: { x: number; y: number }, transform: Transform) {
  return { x: (point.x - transform.x) / transform.k, y: (point.y - transform.y) / transform.k };
}

export function clampScale(scale: number): number {
  return Math.min(Math.max(scale, MIN_SCALE), MAX_SCALE);
}

/**
 * Zoom about a fixed point.
 *
 * The world position under the pointer stays under the pointer, which is what
 * makes wheel-zoom feel like zooming rather than like drifting.
 */
export function zoomAbout(transform: Transform, pointer: { x: number; y: number }, factor: number): Transform {
  const scale = clampScale(transform.k * factor);
  const applied = scale / transform.k;
  return {
    k: scale,
    x: pointer.x - (pointer.x - transform.x) * applied,
    y: pointer.y - (pointer.y - transform.y) * applied,
  };
}

export function pan(transform: Transform, dx: number, dy: number): Transform {
  return { ...transform, x: transform.x + dx, y: transform.y + dy };
}

export interface Positioned {
  id: string;
  x?: number | undefined;
  y?: number | undefined;
}

/**
 * The node under a point, in world coordinates.
 *
 * A linear scan: a personal vault is hundreds of nodes, and a quadtree would be
 * more code to keep correct than it would save.
 */
export function hitTest<T extends Positioned>(
  nodes: readonly T[],
  world: { x: number; y: number },
  radius: number,
): T | null {
  let best: T | null = null;
  let bestDistance = radius * radius;

  for (const node of nodes) {
    if (node.x === undefined || node.y === undefined) continue;
    const dx = node.x - world.x;
    const dy = node.y - world.y;
    const distance = dx * dx + dy * dy;
    // Strictly nearer, so overlapping nodes resolve consistently.
    if (distance <= bestDistance) {
      bestDistance = distance;
      best = node;
    }
  }

  return best;
}

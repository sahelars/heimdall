/**
 * The dark-mode accent (SPEC §15).
 *
 * Light mode has no hue at all, so there is nothing here to choose for it. Dark
 * mode's accent is the user's, and the stylesheet is what decides that: only
 * the dark blocks read `--accent-dark`, so writing it is safe whatever theme is
 * showing and no theme resolution has to happen in TypeScript.
 *
 * `null` means "whatever the stylesheet says", which is how the default stays
 * in styles.css rather than being repeated here — colour lives there and
 * nowhere else, and `colour.test.ts` is what keeps it that way.
 */

/** The custom property the dark blocks read their accent from. */
const PROPERTY = "--accent-dark";

/** The form `<input type="color">` speaks, and the only form worth storing. */
export function isAccent(value: unknown): value is string {
  return typeof value === "string" && /^#[0-9a-f]{6}$/i.test(value);
}

export function isAccentPreference(value: unknown): value is string | null {
  return value === null || isAccent(value);
}

/**
 * Apply a choice to the document.
 *
 * Removing the property rather than writing the default is deliberate: the
 * stylesheet then owns the default outright, so it can move without this file
 * having to hear about it.
 */
export function applyAccent(root: HTMLElement, accent: string | null): void {
  if (accent === null) root.style.removeProperty(PROPERTY);
  else root.style.setProperty(PROPERTY, accent);
}

/**
 * What the accent resolves to right now.
 *
 * The inline override when there is one, the stylesheet's default when there is
 * not — so it is always the value a picker should be showing. An empty string
 * comes back where the stylesheet has not been loaded at all, which is what a
 * DOM-less test environment reports.
 */
export function resolvedAccent(root: HTMLElement): string {
  const value = getComputedStyle(root).getPropertyValue(PROPERTY).trim();
  return isAccent(value) ? value : "";
}

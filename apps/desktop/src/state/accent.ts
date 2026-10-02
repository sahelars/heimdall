/**
 * The accents (SPEC §15).
 *
 * One per theme, because no single colour works on both grounds: white is the
 * right accent on black and invisible on white, and black is the reverse.
 *
 * The stylesheet is what decides which is spent — each theme reads only its own
 * property — so writing either is safe whatever theme is showing and no theme
 * resolution has to happen here.
 *
 * `null` means "whatever the stylesheet says". Removing the property rather
 * than writing a colour keeps the default in styles.css rather than repeating
 * it here: colour lives there and nowhere else, and `colour.test.ts` is what
 * keeps it that way.
 */

/** The two themes an accent can be chosen for. */
export type AccentTheme = "light" | "dark";

/** A choice per theme. `null` is that theme's stylesheet default. */
export type Accents = Record<AccentTheme, string | null>;

/** Nothing chosen: both themes fall back to their end of the palette. */
export const NO_ACCENTS: Accents = { light: null, dark: null };

/** The custom property each theme resolves its accent from. */
const PROPERTY: Record<AccentTheme, string> = {
  light: "--accent-light",
  dark: "--accent-dark",
};

/** That theme's own default, for when nothing has been chosen for it. */
const BASE: Record<AccentTheme, string> = {
  light: "--accent-light-base",
  dark: "--accent-dark-base",
};

/** The form `<input type="color">` speaks, and the only form worth storing. */
export function isAccent(value: unknown): value is string {
  return typeof value === "string" && /^#[0-9a-f]{6}$/i.test(value);
}

/**
 * A stored preference, in either the current shape or the one before it.
 *
 * The accent used to be a single value. Reading that back as dark mode's is
 * what it originally meant, and losing someone's colour because the shape
 * around it changed would be a poor trade for a few lines.
 */
export function isAccentsPreference(value: unknown): value is Accents | string | null {
  if (value === null || isAccent(value)) return true;
  if (typeof value !== "object") return false;
  const candidate = value as Partial<Accents>;
  return (
    Object.keys(candidate).every((key) => key === "light" || key === "dark") &&
    [candidate.light, candidate.dark].every((one) => one === undefined || one === null || isAccent(one))
  );
}

/** Normalize whatever was stored into the current shape. */
export function toAccents(stored: Accents | string | null): Accents {
  if (stored === null) return NO_ACCENTS;
  if (isAccent(stored)) return { light: null, dark: stored };
  return { light: stored.light ?? null, dark: stored.dark ?? null };
}

/**
 * Apply both choices to the document.
 *
 * Removing a property rather than writing the default is deliberate: the
 * stylesheet then owns each default outright, so either can move without this
 * file having to hear about it.
 */
export function applyAccents(root: HTMLElement, accents: Accents): void {
  for (const theme of ["light", "dark"] as const) {
    const accent = accents[theme];
    if (accent === null) root.style.removeProperty(PROPERTY[theme]);
    else root.style.setProperty(PROPERTY[theme], accent);
  }
}

/**
 * A colour as `#rrggbb`, lowercase, from any form a stylesheet can arrive in.
 *
 * The production build's minifier shortens `#000000` to `#000` and `#ffffff`
 * to `#fff`, and a browser may hand a computed colour back as `rgb(…)`. A
 * colour input understands only six-digit hex, so everything read back from
 * the stylesheet comes through here. `null` for anything that is not a colour
 * in one of those forms.
 */
export function toHex(value: string): string | null {
  const text = value.trim().toLowerCase();
  if (/^#[0-9a-f]{6}$/.test(text)) return text;
  const short = /^#([0-9a-f])([0-9a-f])([0-9a-f])$/.exec(text);
  if (short) return `#${short[1]}${short[1]}${short[2]}${short[2]}${short[3]}${short[3]}`;
  const rgb = /^rgba?\(\s*(\d{1,3})[\s,]+(\d{1,3})[\s,]+(\d{1,3})\s*(?:[,/]\s*[\d.]+%?\s*)?\)$/.exec(text);
  if (rgb) {
    const channels = rgb.slice(1, 4).map(Number);
    if (channels.some((channel) => channel > 255)) return null;
    return `#${channels.map((channel) => channel.toString(16).padStart(2, "0")).join("")}`;
  }
  return null;
}

/**
 * The colour a theme uses when nothing has been chosen for it: that theme's
 * own end of the palette, as the stylesheet declares it.
 *
 * Only the base is read. A well that has a choice already knows it from
 * props; reading the chosen property here too would read the document a
 * render behind, because `applyAccents` runs in an effect after the render
 * that asked — so pressing Default left the well showing the colour just
 * cleared.
 *
 * Both bases are declared on `:root`, so this answers for the theme that is
 * not showing as well. An empty string comes back where the stylesheet has
 * not been loaded at all, which is what a DOM-less test environment reports.
 */
export function defaultAccent(root: HTMLElement, theme: AccentTheme): string {
  return toHex(getComputedStyle(root).getPropertyValue(BASE[theme])) ?? "";
}

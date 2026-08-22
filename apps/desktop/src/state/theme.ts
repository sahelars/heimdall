/**
 * Theme resolution (SPEC §15).
 *
 * The system preference is the default and is applied by the stylesheet's media
 * query, so the first paint is already right without waiting for JavaScript. An
 * explicit choice is written to `<html data-theme>`, which the stylesheet
 * answers in both directions.
 */

export type ThemePreference = "system" | "light" | "dark";
export type ResolvedTheme = "light" | "dark";

export const THEME_PREFERENCES: ThemePreference[] = ["system", "light", "dark"];

export function isThemePreference(value: unknown): value is ThemePreference {
  return typeof value === "string" && (THEME_PREFERENCES as string[]).includes(value);
}

/** What the user will actually see, given their choice and the platform's. */
export function resolveTheme(preference: ThemePreference, systemPrefersDark: boolean): ResolvedTheme {
  if (preference === "system") return systemPrefersDark ? "dark" : "light";
  return preference;
}

/**
 * Apply a preference to the document.
 *
 * "System" removes the attribute rather than writing one, so the media query is
 * the only thing deciding — no JavaScript has to stay in sync with the platform.
 */
export function applyTheme(root: HTMLElement, preference: ThemePreference): void {
  if (preference === "system") root.removeAttribute("data-theme");
  else root.setAttribute("data-theme", preference);
}

/**
 * Subscribe to the platform's colour preference.
 *
 * The stylesheet answers it on its own through a media query, but anything
 * drawn rather than styled — a mermaid diagram, whose text metrics are baked
 * into the SVG — has to be told to redraw.
 */
export function watchSystemTheme(onChange: (prefersDark: boolean) => void): () => void {
  const media = window.matchMedia?.("(prefers-color-scheme: dark)");
  if (!media) return () => {};

  const handler = () => onChange(media.matches);
  media.addEventListener?.("change", handler);
  return () => media.removeEventListener?.("change", handler);
}

/** What the platform currently prefers, safely on a host without media queries. */
export function systemPrefersDark(): boolean {
  return window.matchMedia?.("(prefers-color-scheme: dark)")?.matches ?? false;
}

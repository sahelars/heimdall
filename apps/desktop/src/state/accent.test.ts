/**
 * The accent choices and what they resolve to (SPEC §15).
 *
 * There is one accent per theme, so the interesting behaviour is the unset
 * state: with nothing chosen a theme falls back to its own end of the
 * monochrome palette, and each well has to show that — including the well for
 * the theme that is not currently on screen.
 */

import { describe, expect, it } from "vitest";

import {
  applyAccents,
  isAccent,
  isAccentsPreference,
  NO_ACCENTS,
  resolvedAccent,
  toAccents,
} from "./accent";

function root(): HTMLElement {
  const element = document.createElement("div");
  element.style.setProperty("--accent-light-base", "#000000");
  element.style.setProperty("--accent-dark-base", "#ffffff");
  document.body.appendChild(element);
  return element;
}

describe("accent", () => {
  it("accepts only the six-digit hex a colour input speaks", () => {
    expect(isAccent("#ff8800")).toBe(true);
    expect(isAccent("#FF8800")).toBe(true);
    expect(isAccent("#f80")).toBe(false);
    expect(isAccent("rebeccapurple")).toBe(false);
    expect(isAccent(null)).toBe(false);
  });

  it("accepts a stored preference in either shape, and rejects nonsense", () => {
    expect(isAccentsPreference(null)).toBe(true);
    expect(isAccentsPreference({ light: "#000000", dark: "#ffffff" })).toBe(true);
    expect(isAccentsPreference({ light: null, dark: null })).toBe(true);
    expect(isAccentsPreference({ light: "#f80", dark: null })).toBe(false);
    expect(isAccentsPreference({ light: "#000000", other: "#ffffff" })).toBe(false);
    expect(isAccentsPreference("#f80")).toBe(false);
  });

  it("reads a value stored before the accent was split as dark mode's", () => {
    // It is what that value originally meant, and losing someone's colour
    // because the shape around it changed would be a poor trade.
    expect(toAccents("#00ff00")).toEqual({ light: null, dark: "#00ff00" });
    expect(toAccents(null)).toEqual(NO_ACCENTS);
    expect(toAccents({ light: "#ff8800", dark: null })).toEqual({ light: "#ff8800", dark: null });
  });

  it("writes each choice as its own property, and clears rather than writing a default", () => {
    const element = root();

    applyAccents(element, { light: "#ff8800", dark: "#00ff00" });
    expect(element.style.getPropertyValue("--accent-light")).toBe("#ff8800");
    expect(element.style.getPropertyValue("--accent-dark")).toBe("#00ff00");

    // Removed, not set back to a colour: each theme's default belongs to the
    // stylesheet, and the two are different, so there is no value to write.
    applyAccents(element, NO_ACCENTS);
    expect(element.style.getPropertyValue("--accent-light")).toBe("");
    expect(element.style.getPropertyValue("--accent-dark")).toBe("");
  });

  it("clears one theme's choice without disturbing the other", () => {
    const element = root();
    applyAccents(element, { light: "#ff8800", dark: "#00ff00" });
    applyAccents(element, { light: null, dark: "#00ff00" });

    expect(element.style.getPropertyValue("--accent-light")).toBe("");
    expect(element.style.getPropertyValue("--accent-dark")).toBe("#00ff00");
  });

  it("resolves to the choice for that theme when there is one", () => {
    const element = root();
    applyAccents(element, { light: "#ff8800", dark: "#00ff00" });

    expect(resolvedAccent(element, "light")).toBe("#ff8800");
    expect(resolvedAccent(element, "dark")).toBe("#00ff00");
  });

  it("falls back to each theme's own end of the palette", () => {
    // The bug this guards is the one that made two accents necessary: a single
    // shared value is invisible against one of the two backgrounds. Unset, the
    // two must resolve to opposite ends rather than to one colour.
    const element = root();

    expect(resolvedAccent(element, "light")).toBe("#000000");
    expect(resolvedAccent(element, "dark")).toBe("#ffffff");
  });

  it("answers for the theme that is not showing, so both wells can be filled", () => {
    const element = root();
    applyAccents(element, { light: null, dark: "#00ff00" });

    expect(resolvedAccent(element, "light")).toBe("#000000");
    expect(resolvedAccent(element, "dark")).toBe("#00ff00");
  });
});

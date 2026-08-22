import { describe, expect, it } from "vitest";

import { applyTheme, isThemePreference, resolveTheme } from "./theme";

describe("resolving a theme", () => {
  it("follows the platform when the user has not chosen", () => {
    expect(resolveTheme("system", true)).toBe("dark");
    expect(resolveTheme("system", false)).toBe("light");
  });

  it("lets an explicit choice win in both directions", () => {
    expect(resolveTheme("light", true)).toBe("light");
    expect(resolveTheme("dark", false)).toBe("dark");
  });

  it("recognises only the three preferences", () => {
    expect(isThemePreference("dark")).toBe(true);
    expect(isThemePreference("sepia")).toBe(false);
    expect(isThemePreference(null)).toBe(false);
  });
});

describe("applying a theme", () => {
  it("writes an explicit choice and removes it again for system", () => {
    const root = document.createElement("html");

    applyTheme(root, "dark");
    expect(root.getAttribute("data-theme")).toBe("dark");

    applyTheme(root, "light");
    expect(root.getAttribute("data-theme")).toBe("light");

    // System leaves nothing behind, so the media query is the only thing
    // deciding and no JavaScript has to track the platform.
    applyTheme(root, "system");
    expect(root.hasAttribute("data-theme")).toBe(false);
  });
});

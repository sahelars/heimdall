/**
 * The visual rules (SPEC §15), checked against the stylesheet itself.
 *
 * Reading the CSS is the point: these are rules about what the application may
 * look like, and a rendered snapshot would not notice a stray colour on a
 * screen this test never mounts.
 *
 * The palette rule is no longer "no colour". It is "one accent, defined once,
 * spent only on links, the active graph node, and mermaid" — which is still
 * something a machine can check, so it is still checked here rather than
 * trusted to review.
 */

import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { afterAll, beforeAll, describe, expect, it } from "vitest";

const css = readFileSync(join(dirname(fileURLToPath(import.meta.url)), "styles.css"), "utf8");

/** Strip comments so prose about color is not mistaken for a declaration. */
const declarations = css.replace(/\/\*[\s\S]*?\*\//g, "");

describe("typography", () => {
  it("uses the system-safe Helvetica stack", () => {
    expect(declarations).toMatch(
      /--font:\s*"Helvetica Neue",\s*Helvetica,\s*Arial,\s*sans-serif;/,
    );
  });

  it("routes every element through that stack rather than a browser default", () => {
    expect(declarations).toMatch(/body\s*\{[^}]*font-family:\s*var\(--font\)/);
    // Controls do not inherit font by default, so each must ask for it.
    for (const control of [".button", ".input", ".tab"]) {
      const rule = declarations.match(new RegExp(`\\${control}\\s*\\{[^}]*\\}`))?.[0] ?? "";
      expect(rule, control).toMatch(/font-family:\s*inherit/);
    }
  });
});

describe("borders", () => {
  it("squares every corner in the application, not just the ones remembered", () => {
    expect(declarations).toMatch(/\*,[\s\S]*?border-radius:\s*0;/);
  });

  it("never re-introduces a rounded corner", () => {
    const radii = declarations.match(/border-radius:\s*([^;]+);/g) ?? [];
    for (const radius of radii) {
      expect(radius).toMatch(/border-radius:\s*0;/);
    }
  });

  it("never widens a border past a hairline", () => {
    // `border-left-width: 4px` slipped past the shorthand check below for a
    // while: a rule about how heavy borders may be has to cover the longhand
    // properties too.
    for (const width of declarations.match(/border(?:-(?:top|right|bottom|left))?-width:\s*([^;]+);/g) ?? []) {
      expect(width).toMatch(/:\s*(?:0|1px)\s*;/);
    }
  });

  it("draws every border in one colour, the same grey as the pane dividers", () => {
    // Borders are structure, not emphasis. At full contrast every panel, input
    // and dialog shouts — which is what a stark white outline in dark mode did.
    const borders = declarations.match(/(?<!-)border(?:-(?:top|right|bottom|left))?:\s*([^;]+);/g) ?? [];
    for (const border of borders) {
      const value = border.split(":")[1]!.replace(/;\s*$/, "").trim();
      if (value === "none") continue;
      expect(value, border).toMatch(/^1px solid (?:var\(--line\)|var\(--line-soft\)|transparent)$/);
    }
  });

  it("draws every border as 1px solid, including inputs and buttons", () => {
    const borders = declarations.match(/(?<!-)border(?:-(?:top|right|bottom|left))?:\s*([^;]+);/g) ?? [];
    for (const border of borders) {
      const value = border.split(":")[1]!.replace(/;\s*$/, "").trim();
      if (value === "none" || value.startsWith("1px solid")) continue;
      throw new Error(`border is neither none nor 1px solid: ${border}`);
    }

    for (const control of [".input", ".button", ".panel", ".notice", ".snippet"]) {
      const rule = declarations.match(new RegExp(`\\${control}\\s*\\{[^}]*\\}`))?.[0] ?? "";
      expect(rule, control).toMatch(/border:\s*1px solid/);
    }
  });
});

/** The single permitted hue, from the vault template's own green_accent.css. */
const ACCENT = "#00ff00";

/**
 * The only classes allowed to spend the accent.
 *
 * Links, the note title of a linked mention, the row of the note you have open,
 * the active graph node, and mermaid diagrams. Everything else in the
 * application stays greyscale.
 */
const ACCENT_SURFACES = [
  "md-link",
  "property__value",
  "cm-link",
  "backlink",
  "tree__row--current",
  "graph",
  "mermaid",
];

/**
 * Innermost CSS rules, as (selector, body) pairs.
 *
 * The pattern only matches blocks with no braces inside, so an at-rule's
 * prelude is skipped and the rules nested within it come back on their own.
 */
function rules(source: string): { selector: string; body: string }[] {
  return [...source.matchAll(/([^{}]+)\{([^{}]*)\}/g)].map((match) => ({
    selector: match[1]!.trim().replace(/\s+/g, " "),
    body: match[2]!,
  }));
}

/** Whether a hex is a true grey: equal channels, no hue. */
function isGrey(hex: string): boolean {
  const digits = hex.slice(1);
  const expand = digits.length === 3 ? digits.split("").map((d) => d + d) : digits.match(/../g)!;
  const [r, g, b] = expand.slice(0, 3).map((pair) => parseInt(pair, 16));
  return r === g && g === b;
}

/** Every custom property a block declares, normalised and sorted. */
function tokens(block: string): string[] {
  return (block.match(/--[a-z-]+:\s*[^;]+;/g) ?? [])
    .map((line) => line.replace(/\s+/g, " ").trim())
    .sort();
}

/**
 * Where text may be taken from.
 *
 * The note in either of its two forms, and the fields and commands that exist
 * to be typed into or copied out of. A selection anywhere else is a drag that
 * painted the file tree grey.
 */
const SELECTABLE = [".editor", ".preview", ".snippet", "input", "textarea", "[contenteditable"];

describe("selection", () => {
  it("turns selection off for the whole application", () => {
    const body = (rules(declarations).find((rule) => rule.selector === "body")?.body) ?? "";
    // Prefixed as well: WebKit is what this ships inside.
    expect(body).toMatch(/-webkit-user-select:\s*none;/);
    expect(body).toMatch(/(?<!-)user-select:\s*none;/);
  });

  it("gives it back only to the note, and to what is there to be copied", () => {
    for (const rule of rules(declarations)) {
      if (!/user-select:\s*text/.test(rule.body)) continue;
      const allowed = rule.selector
        .split(",")
        .every((part) => SELECTABLE.some((surface) => part.trim().startsWith(surface)));
      expect(allowed, `"${rule.selector}" makes something else selectable`).toBe(true);
    }
  });
});

/**
 * Cursors that are not the hand, and the one surface each is allowed on.
 *
 * Everything else that can be pressed gets `pointer`, and everything that
 * cannot, because it is disabled, gets `not-allowed`. Anything outside this
 * table is a control that quietly went back to an arrow.
 */
const CURSOR_EXCEPTIONS: Record<string, string> = {
  "col-resize": ".divider",
  grab: ".graph__canvas",
  grabbing: ".graph__canvas",
  // An external link is a `<span>` with a title, not somewhere to click yet.
  default: ".md-link--external",
};

/**
 * One of each kind of thing the pointer can be over, and what it should read as.
 *
 * Resolved rather than read: which cursor wins is a cascade question — an
 * element selector against a class, a `:disabled` against the class that gave
 * the control its hand — and grepping the stylesheet for `cursor: pointer`
 * answers none of it. Every rule this file states was already stated somewhere
 * before the fix; what was missing was the cascade coming out right.
 */
const CURSORS: { markup: string; cursor: string; because: string }[] = [
  { markup: `<button class="button" id="target">Save</button>`, cursor: "pointer", because: "a button" },
  {
    markup: `<button class="icon-button" id="target" aria-label="Back"></button>`,
    cursor: "pointer",
    because: "an icon button",
  },
  {
    markup: `<button class="button" disabled><span id="target">Save</span></button>`,
    cursor: "not-allowed",
    because: "the label inside a disabled button, which is what the pointer is actually over",
  },
  {
    markup: `<button class="icon-button" disabled><svg id="target"></svg></button>`,
    cursor: "not-allowed",
    because: "the icon inside a disabled icon button",
  },
  { markup: `<button class="md-link" id="target">Note</button>`, cursor: "pointer", because: "a wikilink" },
  {
    markup: `<span class="md-link md-link--external" id="target">https://example.com</span>`,
    cursor: "default",
    because: "an external link, which is not somewhere to click",
  },
  {
    markup: `<div class="tree__row" role="treeitem" id="target"></div>`,
    cursor: "pointer",
    because: "a file tree row, which the ARIA pattern makes a div",
  },
  {
    markup: `<label class="choice__option"><input type="radio" id="target"></label>`,
    cursor: "pointer",
    because: "the radio itself, not just the words beside it",
  },
  {
    markup: `<label class="field__label" for="x" id="target">Vault name</label><input id="x">`,
    cursor: "pointer",
    because: "a field label, which focuses its field and which WebKit would otherwise leave an arrow",
  },
  {
    markup: `<div class="divider" role="separator" id="target"></div>`,
    cursor: "col-resize",
    because: "a pane divider, which is dragged rather than pressed",
  },
  {
    markup: `<canvas class="graph__canvas" id="target"></canvas>`,
    cursor: "grab",
    because: "the graph, which is panned",
  },
];

describe("cursors", () => {
  const style = document.createElement("style");
  style.textContent = css;

  beforeAll(() => document.head.append(style));
  afterAll(() => {
    style.remove();
    document.body.innerHTML = "";
  });

  it.each(CURSORS)("reads as $cursor over $because", ({ markup, cursor }) => {
    document.body.innerHTML = markup;
    expect(getComputedStyle(document.getElementById("target")!).cursor).toBe(cursor);
  });

  it("never quietly hands a control back its arrow", () => {
    // The table above covers what exists today; this covers what gets added.
    for (const rule of rules(declarations)) {
      const cursor = rule.body.match(/cursor:\s*([^;]+);/)?.[1]?.trim();
      if (!cursor || cursor === "pointer" || cursor === "not-allowed") continue;

      const surface = CURSOR_EXCEPTIONS[cursor];
      expect(surface, `"${rule.selector}" uses an unlisted cursor: ${cursor}`).toBeDefined();
      expect(
        rule.selector.includes(surface!),
        `"${rule.selector}" is not the surface ${cursor} belongs to`,
      ).toBe(true);
    }
  });
});

describe("pane dividers", () => {
  it("does not light up when the pointer crosses one", () => {
    // `col-resize` already says the rule can be dragged, and it says it from
    // the whole track. Repainting the hairline as well flashed two lines across
    // the window on every pass of the mouse.
    for (const rule of rules(declarations)) {
      if (!rule.selector.includes(".divider") || !rule.selector.includes(":hover")) continue;
      expect(rule.body, `"${rule.selector}" restyles a divider on hover`).not.toMatch(
        /background|border|color|opacity/,
      );
    }
  });

  it("still shows itself to a divider reached with the keyboard", () => {
    // There is no pointer over a focused divider to say where it is.
    const rule = rules(declarations).find((candidate) =>
      candidate.selector.includes(".divider:focus-visible::after"),
    );
    expect(rule?.body).toMatch(/background:\s*var\(--fg\)/);
  });
});

describe("the settings sheet", () => {
  it("draws no rule under its title", () => {
    // The tab strip below draws its own, and two hairlines a row apart boxed
    // the close button into a corner of its own.
    const header = rules(declarations).find((rule) => rule.selector === ".modal__header");
    expect(header, ".modal__header is gone").toBeDefined();
    expect(header!.body).not.toMatch(/border/);
  });

  it("shows no focus ring on the sheet itself, which is a container", () => {
    const rule = rules(declarations).find((candidate) => candidate.selector === ".modal:focus");
    expect(rule?.body).toMatch(/outline:\s*none/);
  });
});

describe("palette", () => {
  it("is pure black on white in light mode and the exact inverse in dark", () => {
    const root = declarations.match(/:root\s*\{[^}]*\}/)?.[0] ?? "";
    expect(root).toMatch(/--bg:\s*#ffffff;/);
    expect(root).toMatch(/--fg:\s*#000000;/);

    const dark = declarations.match(/@media \(prefers-color-scheme: dark\)\s*\{[\s\S]*?\n\}/)?.[0] ?? "";
    expect(dark).toMatch(/--bg:\s*#000000;/);
    expect(dark).toMatch(/--fg:\s*#ffffff;/);
  });

  it("follows the system preference rather than hard-coding one mode", () => {
    expect(declarations).toContain("prefers-color-scheme: dark");
    expect(declarations).toMatch(/color-scheme:\s*light dark/);
  });

  it("lets a user override the system preference in both directions", () => {
    expect(declarations).toMatch(/:root\[data-theme="dark"\]\s*\{/);
    expect(declarations).toMatch(/:root\[data-theme="light"\]\s*\{/);
    // Without the :not(), a machine set to dark would ignore an explicit
    // choice of light, because the media query would keep winning.
    expect(declarations).toMatch(/:root:not\(\[data-theme="light"\]\)/);
  });

  it("carries one accent and no other hue: every remaining value is a true grey", () => {
    const hexes = declarations.match(/#[0-9a-fA-F]{3,8}\b/g) ?? [];
    expect(hexes.length).toBeGreaterThan(0);

    for (const hex of hexes) {
      if (hex.toLowerCase() === ACCENT) continue;
      expect(isGrey(hex), `${hex} is neither a grey nor the one accent`).toBe(true);
    }
  });

  it("writes the accent only as --accent, so there is one place to change it", () => {
    const literals = declarations.match(new RegExp(ACCENT, "gi")) ?? [];
    const definitions = (declarations.match(/--accent:\s*#[0-9a-fA-F]{3,8};/g) ?? []).filter(
      (line) => line.toLowerCase().includes(ACCENT),
    );
    expect(definitions.length).toBeGreaterThan(0);
    expect(literals.length, "the accent is written somewhere other than --accent").toBe(
      definitions.length,
    );

    // Light mode has no hue at all: its links are black (SPEC §15).
    const root = declarations.match(/:root\s*\{[^}]*\}/)?.[0] ?? "";
    expect(root).toMatch(/--accent:\s*#000000;/);
  });

  it("spends the accent only on links, the open note, the active graph node, and mermaid", () => {
    for (const rule of rules(declarations)) {
      if (!/var\(--accent\)|var\(--link\)/.test(rule.body)) continue;
      if (rule.selector.startsWith(":root")) continue;
      const allowed = ACCENT_SURFACES.some((surface) => rule.selector.includes(`.${surface}`));
      expect(allowed, `"${rule.selector}" is not a surface the accent may be spent on`).toBe(true);
    }
  });

  it("keeps the two dark blocks identical, so an override cannot drift", () => {
    const media = declarations.match(/@media \(prefers-color-scheme: dark\)\s*\{[\s\S]*?\n\}/)?.[0] ?? "";
    const override = declarations.match(/:root\[data-theme="dark"\]\s*\{[^}]*\}/)?.[0] ?? "";
    expect(tokens(override).length).toBeGreaterThan(5);
    expect(tokens(media)).toEqual(tokens(override));
  });

  it("uses no colour function that could smuggle a second hue in", () => {
    expect(declarations).not.toMatch(/\b(?:rgb|rgba|hsl|hsla|oklch|lab|color)\(/);
  });
});

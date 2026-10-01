/**
 * The visual rules (SPEC §15), checked against the stylesheet itself.
 *
 * Reading the CSS is the point: these are rules about what the application may
 * look like, and a rendered snapshot would not notice a stray colour on a
 * screen this test never mounts.
 *
 * The palette rule is "no hue in this file at all": light mode's accent is
 * black, dark mode's is the user's and arrives at runtime, and both are spent
 * only on links, the open note, the active graph node, and mermaid. All of
 * that is still something a machine can check, so it is still checked here
 * rather than trusted to review.
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

describe("keyboard focus", () => {
  it("draws a button's focus ring in ink rather than in the system's blue", () => {
    // WebKit's default ring is the one colour in the application that is not
    // the application's: a hue, on a surface the palette says is greyscale.
    for (const control of [".button:focus-visible", ".icon-button:focus-visible"]) {
      const rule = rules(declarations).find((candidate) =>
        candidate.selector.split(",").some((part) => part.trim() === control),
      );
      expect(rule, `${control} has no rule, so the system ring shows through`).toBeDefined();
      expect(rule!.body).toMatch(/outline:\s*1px solid var\(--fg\);/);
    }
  });

  it("never hides a focus ring without putting something in its place", () => {
    // `outline: none` is allowed only where the element is a container nobody
    // is being pointed at, or where a rule of its own is drawn instead.
    const REPLACED = [
      ".divider:focus-visible",
      ".modal:focus",
      ".note__title-field:focus",
      ".switcher__input:focus",
      ".preview__title-field:focus",
    ];
    for (const rule of rules(declarations)) {
      if (!/outline:\s*none/.test(rule.body)) continue;
      expect(
        REPLACED.some((surface) => rule.selector.includes(surface)),
        `"${rule.selector}" hides a focus ring with nothing in its place`,
      ).toBe(true);
    }
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

  it("keeps one size from tab to tab", () => {
    // Sized to its content, the sheet jumped every time a tab changed.
    const sheet = rules(declarations).find((rule) => rule.selector === ".modal");
    expect(sheet?.body).toMatch(/(^|[\s;])height:\s*86vh;/);
    expect(sheet?.body).not.toMatch(/max-height/);
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

  it("carries no hue at all: the accent is the user's, and is never written here", () => {
    const hexes = declarations.match(/#[0-9a-fA-F]{3,8}\b/g) ?? [];
    expect(hexes.length).toBeGreaterThan(0);

    for (const hex of hexes) {
      expect(isGrey(hex), `${hex} is not a grey`).toBe(true);
    }
  });

  it("gives each theme its own accent, resolved from its own property", () => {
    // One accent per theme (SPEC §15): a single shared one is invisible against
    // one of the two backgrounds. Each theme reads only its own property, and
    // the fallback beside it is that theme's monochrome default.
    const root = declarations.match(/:root\s*\{[^}]*\}/)?.[0] ?? "";
    expect(root).toMatch(/--accent:\s*var\(--accent-light,\s*var\(--accent-light-base\)\);/);

    const media = declarations.match(/@media \(prefers-color-scheme: dark\)\s*\{[\s\S]*?\n\}/)?.[0] ?? "";
    const override = declarations.match(/:root\[data-theme="dark"\]\s*\{[^}]*\}/)?.[0] ?? "";
    for (const block of [media, override]) {
      expect(block).toMatch(/--accent:\s*var\(--accent-dark,\s*var\(--accent-dark-base\)\);/);
    }
  });

  it("states both defaults on :root, as opposite ends of the palette", () => {
    // On `:root` rather than inside their own theme blocks, so the picker can
    // read the default for the theme that is not currently showing — it shows a
    // well for each. Opposite ends is the point: black on white, white on black.
    const root = declarations.match(/:root\s*\{[^}]*\}/)?.[0] ?? "";
    expect(root).toMatch(/--accent-light-base:\s*#000000;/);
    expect(root).toMatch(/--accent-dark-base:\s*#ffffff;/);
  });

  it("spends an accent in light mode too, not only in dark", () => {
    // The regression this guards: the accent used to be read only by the dark
    // blocks, so a colour chosen in Settings did nothing at all in light mode.
    const root = declarations.match(/:root\s*\{[^}]*\}/)?.[0] ?? "";
    expect(root).toContain("var(--accent-light");
  });

  it("never declares the chosen accents, so each theme's fallback is its default", () => {
    // The whole mechanism. An unset custom property falls through to its var()
    // fallback, which is how the property Settings writes can be absent and
    // still leave a working colour behind. Declaring either here would make a
    // choice impossible to clear.
    for (const property of ["--accent-light", "--accent-dark"]) {
      expect(declarations, `${property} must not be declared`).not.toMatch(
        new RegExp(`${property}:\\s*[^;]+;`),
      );
    }

    // And every read carries a fallback, or the unset state resolves to nothing
    // at all rather than to a colour.
    const reads = declarations.match(/var\(--accent-(?:light|dark)[^;]*\)/g) ?? [];
    expect(reads.length).toBeGreaterThan(0);
    for (const read of reads) {
      expect(read, `${read} has no fallback`).toMatch(
        /var\(--accent-(light|dark),\s*var\(--accent-\1-base\)/,
      );
    }

    // Only the theme blocks read them. Everything else goes through `--accent`,
    // so there is exactly one place per theme that decides what the accent is
    // and no rule can reach past it for the raw choice.
    for (const rule of rules(declarations)) {
      if (!/var\(--accent-(?:light|dark),/.test(rule.body)) continue;
      expect(
        rule.selector.startsWith(":root"),
        `"${rule.selector}" reads a chosen accent instead of --accent`,
      ).toBe(true);
    }
  });

  it("spends the accent only on links, the open note, the active graph node, and mermaid", () => {
    for (const rule of rules(declarations)) {
      // The theme properties as well as `--accent`: a new surface must not be
      // able to spend the accent by reaching past it for a token behind it.
      if (!/var\(--accent[a-z-]*\)|var\(--link\)/.test(rule.body)) continue;
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

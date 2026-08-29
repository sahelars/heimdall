/**
 * The source view's decorations, tested headlessly.
 *
 * `EditorState` is pure, so this needs none of the layout and text measurement
 * jsdom cannot provide — mounting an `EditorView` would.
 */

import { EditorState } from "@codemirror/state";
import { describe, expect, it } from "vitest";

import { testables } from "./editorExtensions";

/** The ranges one class was applied to, in document order. */
function marked(doc: string, className: string): { from: number; to: number }[] {
  const set = testables.markerDecorations(EditorState.create({ doc }));
  const found: { from: number; to: number }[] = [];
  set.between(0, doc.length, (from, to, value) => {
    if (value.spec.class === className) found.push({ from, to });
  });
  return found;
}

const marks = (doc: string) => marked(doc, "cm-frontmatter");
const rules = (doc: string) => marked(doc, "cm-rule").map(({ from, to }) => doc.slice(from, to));

describe("dimming frontmatter", () => {
  it("covers the block and stops at its closing rule", () => {
    const doc = '---\nlinks:\n  - "[[profile]]"\n---\n\n# Body\n';
    const [range] = marks(doc);

    expect(range).toBeDefined();
    expect(range!.from).toBe(0);
    expect(doc.slice(range!.from, range!.to)).toBe('---\nlinks:\n  - "[[profile]]"\n---');
  });

  it("covers a block written under the note's heading, where a titled note puts it", () => {
    // The heading is drawn above the editor, so what the editor holds opens
    // with the block — but a note edited elsewhere may well not.
    const doc = "# Title\n\n---\ntags: [a]\n---\n\nBody\n";
    const [range] = marks(doc);

    expect(range).toBeDefined();
    expect(doc.slice(range!.from, range!.to)).toBe("---\ntags: [a]\n---");
  });

  it("leaves a lone rule alone", () => {
    expect(marks("# Body\n\n---\n")).toEqual([]);
  });

  it("leaves a pair of rules holding prose alone", () => {
    // `---` is a horizontal rule too, and dimming everything between two of
    // them would grey out a paragraph the user can see is a paragraph.
    expect(marks("# T\n\n---\n\nA break.\n\n---\n\nMore.\n")).toEqual([]);
  });

  it("does not dim the rest of the file when the block is unterminated", () => {
    // A note whose body opens with a horizontal rule is a note, which is the
    // same rule the vault applies on the way in.
    expect(marks("---\nnot really yaml\n\nstill the body\n")).toEqual([]);
  });

  it("handles an empty document", () => {
    expect(marks("")).toEqual([]);
  });
});

/**
 * A rule keeps one colour while it is being typed.
 *
 * The grammar reads `-` as a list marker and `---` as a horizontal rule, both
 * of which the highlight style dims — but `--` in between is neither, and came
 * back as ordinary text at full contrast. The mark spans the run at every
 * length, so the dashes do not flash on the way to three.
 */
describe("holding a rule's colour", () => {
  it.each(["-", "--", "---", "----", "-----"])("marks a line of %s", (dashes) => {
    expect(rules(`# T\n\n${dashes}\n`)).toEqual([dashes]);
  });

  it("marks the dashes and not the whitespace around them", () => {
    expect(rules("# T\n\n  --  \n")).toEqual(["--"]);
  });

  it("leaves a list item alone, which is a dash with something after it", () => {
    expect(rules("# T\n\n- item\n-- item\n")).toEqual([]);
  });

  it("leaves a setext underline marked too, since it is still a line of dashes", () => {
    // Not a rule to the grammar — it underlines the paragraph above — but the
    // dashes are dim either way, so the mark agrees with the highlight style.
    expect(rules("Heading\n---\n")).toEqual(["---"]);
  });

  it("marks every rule in a note, not only the first", () => {
    expect(rules("# T\n\n---\n\nA break.\n\n--\n\nMore.\n")).toEqual(["---", "--"]);
  });

  it("leaves the frontmatter fences to the block's own grey", () => {
    const doc = "---\ntags: [a]\n---\n\n---\n";
    expect(rules(doc)).toEqual(["---"]);
    // The one it did mark is the rule under the block, not either fence.
    expect(marked(doc, "cm-rule")[0]!.from).toBeGreaterThan(marks(doc)[0]!.to);
  });

  it("measures past a byte order mark rather than through it", () => {
    expect(rules("\ufeff---\n")).toEqual(["---"]);
  });

  it("handles an empty document", () => {
    expect(rules("")).toEqual([]);
  });
});

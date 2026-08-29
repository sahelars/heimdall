/**
 * The source view's decorations, tested headlessly.
 *
 * `EditorState` is pure, so this needs none of the layout and text measurement
 * jsdom cannot provide — mounting an `EditorView` would.
 */

import { EditorState } from "@codemirror/state";
import { describe, expect, it } from "vitest";

import { testables } from "./editorExtensions";

function marks(doc: string): { from: number; to: number }[] {
  const set = testables.frontmatterDecoration(EditorState.create({ doc }));
  const found: { from: number; to: number }[] = [];
  set.between(0, doc.length, (from, to) => {
    found.push({ from, to });
  });
  return found;
}

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

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

  it("leaves a note that does not open with a block alone", () => {
    expect(marks("# Body\n\n---\n")).toEqual([]);
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

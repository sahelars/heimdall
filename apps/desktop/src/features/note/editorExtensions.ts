/**
 * The CodeMirror configuration behind the source view.
 *
 * Extensions are picked one by one rather than pulled in through `basicSetup`,
 * which would add search, linting, folding, and autocompletion this view has no
 * use for.
 *
 * Every colour is a CSS custom property read from the stylesheet. That is what
 * lets the theme switch without rebuilding the editor, and what keeps the whole
 * palette inside the one file `design.test.ts` checks.
 */

import { defaultKeymap, history, historyKeymap } from "@codemirror/commands";
import { markdown } from "@codemirror/lang-markdown";
import { HighlightStyle, syntaxHighlighting } from "@codemirror/language";
import { EditorState, StateField, type Extension } from "@codemirror/state";
import { Decoration, EditorView, keymap, type DecorationSet } from "@codemirror/view";
import { tags } from "@lezer/highlight";

import { findFrontmatterBlock, type Block } from "../../markdown/frontmatter";

/**
 * Source-mode highlighting.
 *
 * `###` and the other markers are `processingInstruction` in the Markdown
 * grammar, so dimming them next to bold heading text — the look in the edit-mode
 * screenshot — is one entry rather than a bespoke tokenizer.
 */
const highlight = HighlightStyle.define([
  { tag: tags.heading, fontWeight: "700" },
  { tag: tags.processingInstruction, color: "var(--faint)" },
  { tag: tags.strong, fontWeight: "700" },
  { tag: tags.emphasis, fontStyle: "italic" },
  { tag: tags.link, color: "var(--link)" },
  { tag: tags.url, color: "var(--link)" },
  { tag: tags.monospace, color: "var(--muted)" },
  { tag: tags.quote, color: "var(--muted)" },
  { tag: tags.contentSeparator, color: "var(--faint)" },
  { tag: tags.list, color: "var(--muted)" },
]);

const theme = EditorView.theme({
  "&": {
    backgroundColor: "var(--bg)",
    color: "var(--fg)",
    height: "100%",
    fontSize: "14px",
  },
  ".cm-content": {
    fontFamily: "var(--mono)",
    padding: "8px 0 40vh 0",
    caretColor: "var(--fg)",
  },
  "&.cm-focused": { outline: "none" },
  ".cm-gutters": { display: "none" },
  ".cm-activeLine": { backgroundColor: "transparent" },
  ".cm-cursor, .cm-dropCursor": { borderLeftColor: "var(--fg)" },
  "&.cm-focused .cm-selectionBackground, ::selection": {
    backgroundColor: "var(--bg-selected)",
  },
  ".cm-frontmatter": { color: "var(--muted)" },
  ".cm-rule": { color: "var(--faint)" },
  ".cm-scroller": { overflow: "auto", lineHeight: "1.6" },
});

/**
 * The two things the grammar does not say for itself: the `---` block, and a
 * rule part-way through being typed.
 *
 * `@codemirror/lang-markdown` has no frontmatter support, and writing a Lezer
 * block parser for it would be a lot of machinery for what the screenshot asks
 * for, which is only that the block reads as metadata rather than as prose.
 *
 * Which lines are the block is `findFrontmatterBlock`'s answer and not this
 * file's. Deciding it twice is how the editor came to disagree with the preview
 * about a byte order mark and about a fence with a space after it. Both marks
 * come out of one field for the same reason: the block is found once per edit.
 */
const markerField = StateField.define<DecorationSet>({
  create: (state) => markerDecorations(state),
  update: (value, transaction) =>
    transaction.docChanged ? markerDecorations(transaction.state) : value,
  provide: (field) => EditorView.decorations.from(field),
});

const frontmatterMark = Decoration.mark({ class: "cm-frontmatter" });
const ruleMark = Decoration.mark({ class: "cm-rule" });

/** A line that is nothing but dashes — a rule, or one on its way to being one. */
const DASHES = /^([ \t]*)(-+)[ \t]*$/;

/** The document's lines, with a leading byte order mark taken off the first. */
function textLines(state: EditorState): string[] {
  const lines: string[] = [];
  for (let line = 1; line <= state.doc.lines; line += 1) lines.push(state.doc.line(line).text);
  // Only for the matching: stripping it from the document would shift every
  // offset the decoration is measured in.
  if (lines[0]?.startsWith("\ufeff")) lines[0] = lines[0].slice(1);
  return lines;
}

function frontmatterRange(state: EditorState, block: Block) {
  return frontmatterMark.range(
    state.doc.line(block.start + 1).from,
    state.doc.line(block.end + 1).to,
  );
}

/**
 * Hold a rule's dashes at one colour from the first keystroke to the last.
 *
 * The grammar changes its mind twice on the way to `---`: a lone `-` is a
 * `ListMark` and a `---` is a `HorizontalRule`, both of which this file dims,
 * but `--` is neither and comes back as ordinary paragraph text — so the
 * dashes flashed to full contrast between the second keystroke and the third.
 * Marking the whole run keeps every stage the colour the finished rule is.
 *
 * Lines inside the frontmatter block are left out: its fences are `---` too,
 * and the block already has a grey of its own that says it is metadata.
 */
function markerDecorations(state: EditorState): DecorationSet {
  const lines = textLines(state);
  const block = findFrontmatterBlock(lines);
  const ranges = block ? [frontmatterRange(state, block)] : [];

  for (let index = 0; index < lines.length; index += 1) {
    if (block && index >= block.start && index <= block.end) {
      index = block.end;
      continue;
    }
    const dashes = DASHES.exec(lines[index]!);
    if (!dashes) continue;
    const line = state.doc.line(index + 1);
    // Measured against the line the document actually holds, so a byte order
    // mark `textLines` took off the first line does not shift the range.
    const from = line.from + (line.text.length - lines[index]!.length) + dashes[1]!.length;
    ranges.push(ruleMark.range(from, from + dashes[2]!.length));
  }

  // Sorted here rather than by construction: the block is pushed first and a
  // rule above it would come before it in the document.
  return ranges.length === 0 ? Decoration.none : Decoration.set(ranges, true);
}

export interface EditorConfig {
  onChange: (text: string) => void;
  onSave: () => void;
  /**
   * Whether the note can be written back.
   *
   * A read-only note still gets an editor so its source can be read and copied,
   * but not a caret: letting someone type a paragraph into an entry that
   * `write_entry` will never be asked to save is worse than not offering it.
   */
  editable: boolean;
}

export function editorExtensions({ onChange, onSave, editable }: EditorConfig): Extension[] {
  return [
    EditorState.readOnly.of(!editable),
    EditorView.editable.of(editable),
    history(),
    keymap.of([
      {
        key: "Mod-s",
        run: () => {
          onSave();
          return true;
        },
      },
      ...defaultKeymap,
      ...historyKeymap,
    ]),
    markdown(),
    syntaxHighlighting(highlight),
    markerField,
    theme,
    EditorView.lineWrapping,
    EditorView.updateListener.of((update) => {
      if (update.docChanged) onChange(update.state.doc.toString());
    }),
  ];
}

/** Exported for tests, which build a state rather than mounting a view. */
export const testables = { markerDecorations };

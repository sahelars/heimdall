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
  ".cm-scroller": { overflow: "auto", lineHeight: "1.6" },
});

/**
 * Dim the leading `---` block.
 *
 * `@codemirror/lang-markdown` has no frontmatter support, and writing a Lezer
 * block parser for it would be a lot of machinery for what the screenshot asks
 * for, which is only that the block reads as metadata rather than as prose.
 */
const frontmatterField = StateField.define<DecorationSet>({
  create: (state) => frontmatterDecoration(state),
  update: (value, transaction) =>
    transaction.docChanged ? frontmatterDecoration(transaction.state) : value,
  provide: (field) => EditorView.decorations.from(field),
});

const frontmatterMark = Decoration.mark({ class: "cm-frontmatter" });

function frontmatterDecoration(state: EditorState): DecorationSet {
  if (state.doc.lines === 0) return Decoration.none;
  if (state.doc.line(1).text.trimEnd() !== "---") return Decoration.none;

  for (let line = 2; line <= state.doc.lines; line += 1) {
    const candidate = state.doc.line(line);
    if (candidate.text.trimEnd() !== "---") continue;
    return Decoration.set([frontmatterMark.range(0, candidate.to)]);
  }
  // An unterminated block is not a block — the same rule the vault applies.
  return Decoration.none;
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
    frontmatterField,
    theme,
    EditorView.lineWrapping,
    EditorView.updateListener.of((update) => {
      if (update.docChanged) onChange(update.state.doc.toString());
    }),
  ];
}

/** Exported for tests, which build a state rather than mounting a view. */
export const testables = { frontmatterDecoration };

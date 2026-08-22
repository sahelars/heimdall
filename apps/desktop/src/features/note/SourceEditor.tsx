/**
 * The CodeMirror host.
 *
 * The view is created once and then reconciled: an external replacement — a
 * reload after a conflict, or opening another note — dispatches a change, while
 * ordinary typing does not round-trip through React state.
 */

import { useEffect, useRef } from "react";
import { EditorState } from "@codemirror/state";
import { EditorView } from "@codemirror/view";

import { editorExtensions } from "./editorExtensions";

interface SourceEditorProps {
  /** Identifies the open note, so switching notes rebuilds the state. */
  path: string;
  value: string;
  editable: boolean;
  onChange: (text: string) => void;
  onSave: () => void;
}

export function SourceEditor({ path, value, editable, onChange, onSave }: SourceEditorProps) {
  const host = useRef<HTMLDivElement | null>(null);
  const view = useRef<EditorView | null>(null);
  // Kept in a ref so changing the handler does not tear down the editor and
  // lose the cursor.
  const handlers = useRef({ onChange, onSave });
  handlers.current = { onChange, onSave };

  useEffect(() => {
    if (!host.current) return;

    const editor = new EditorView({
      state: EditorState.create({
        doc: value,
        extensions: editorExtensions({
          onChange: (text) => handlers.current.onChange(text),
          onSave: () => handlers.current.onSave(),
          editable,
        }),
      }),
      parent: host.current,
    });
    view.current = editor;
    // Someone who clicked "Edit" means to type. Without this the caret stays
    // wherever it was and the first keystrokes go nowhere — most obviously
    // after creating a note, which opens straight into an empty editor.
    if (editable) editor.focus();

    return () => {
      editor.destroy();
      view.current = null;
    };
    // Rebuilt per note: a fresh document means a fresh undo history, which is
    // what someone switching notes expects.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [path, editable]);

  useEffect(() => {
    const editor = view.current;
    if (!editor) return;
    const current = editor.state.doc.toString();
    // Only when something other than typing changed the text; dispatching on
    // every keystroke would fight the cursor.
    if (current === value) return;
    editor.dispatch({ changes: { from: 0, to: current.length, insert: value } });
  }, [value]);

  return <div className="editor" ref={host} data-testid="source-editor" />;
}

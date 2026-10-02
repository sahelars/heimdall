/**
 * The CodeMirror host.
 *
 * The view is created once and then reconciled: an external replacement — a
 * reload after a conflict, or opening another note — dispatches a change, while
 * ordinary typing does not round-trip through React state.
 */

import { useEffect, useRef, type MutableRefObject } from "react";
import { EditorState } from "@codemirror/state";
import { EditorView } from "@codemirror/view";

import { editability, editableCompartment, editorExtensions } from "./editorExtensions";

interface SourceEditorProps {
  /** Identifies the open note, so switching notes rebuilds the state. */
  path: string;
  value: string;
  editable: boolean;
  onChange: (text: string) => void;
  onSave: () => void;
  /** Filled with a way to put the caret in the body — the title field uses it. */
  focusHandle?: MutableRefObject<(() => void) | null>;
}

export function SourceEditor({
  path,
  value,
  editable,
  onChange,
  onSave,
  focusHandle,
}: SourceEditorProps) {
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
    if (focusHandle) focusHandle.current = () => editor.focus();
    // Someone who clicked "Edit" means to type. Without this the caret stays
    // wherever it was and the first keystrokes go nowhere — most obviously
    // after creating a note, which opens straight into an empty editor.
    if (editable) editor.focus();

    return () => {
      editor.destroy();
      view.current = null;
      if (focusHandle) focusHandle.current = null;
    };
    // Rebuilt per note: a fresh document means a fresh undo history, which is
    // what someone switching notes expects. Not per `editable` — locking the
    // open note is reconfigured below, keeping the history and the caret.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [path]);

  useEffect(() => {
    const editor = view.current;
    if (!editor || editor.state.readOnly === !editable) return;
    editor.dispatch({ effects: editableCompartment.reconfigure(editability(editable)) });
  }, [editable]);

  useEffect(() => {
    const editor = view.current;
    if (!editor) return;
    const current = editor.state.doc.toString();
    // Only when something other than typing changed the text; dispatching on
    // every keystroke would fight the cursor.
    if (current === value) return;
    // Only the part that actually differs. Replacing the whole document would
    // map every selection to its end, so a change above the caret — the title
    // being lifted into the header, a conflict resolved with the other side —
    // would send the caret to the bottom of the note.
    editor.dispatch({ changes: narrow(current, value) });
  }, [value]);

  return <div className="editor" ref={host} data-testid="source-editor" />;
}

/** The one span in which two texts differ, as a CodeMirror change. */
function narrow(from: string, to: string): { from: number; to: number; insert: string } {
  let start = 0;
  while (start < from.length && start < to.length && from[start] === to[start]) start += 1;

  let end = 0;
  while (
    end < from.length - start &&
    end < to.length - start &&
    from[from.length - 1 - end] === to[to.length - 1 - end]
  ) {
    end += 1;
  }

  return { from: start, to: from.length - end, insert: to.slice(start, to.length - end) };
}

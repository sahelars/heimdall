/**
 * The note's title, which is its filename.
 *
 * There is no separate title to keep in step: a note is always called
 * something, and that something is the name of the file. Editing the heading
 * renames the file, and renaming the file changes the heading, because they are
 * the same fact shown once.
 *
 * That is the conventional model, and it is why this is an editable heading rather
 * than a text field: it should read as the title of the note, not as a form.
 */

import { useEffect, useRef, useState } from "react";

interface NoteTitleProps {
  /** The current name, without its extension. */
  title: string;
  /** Null when the note cannot be renamed from here. */
  onRename: ((name: string) => void) | null;
}

export function NoteTitle({ title, onRename }: NoteTitleProps) {
  const [draft, setDraft] = useState(title);
  const [editing, setEditing] = useState(false);
  const field = useRef<HTMLInputElement | null>(null);
  /**
   * Set by Escape, read by the blur that Escape causes.
   *
   * Blurring is what commits, and `setDraft` has not flushed by the time the
   * blur handler runs — so without this, abandoning an edit would rename the
   * file to whatever was abandoned.
   */
  const abandoning = useRef(false);

  // Follow the file when it is renamed from anywhere else — the tree's context
  // menu, or another window.
  useEffect(() => {
    if (!editing) setDraft(title);
  }, [title, editing]);

  const commit = () => {
    setEditing(false);
    if (abandoning.current) {
      abandoning.current = false;
      setDraft(title);
      return;
    }

    const name = draft.trim();
    if (!name || name === title) {
      setDraft(title);
      return;
    }
    onRename?.(name);
  };

  if (!onRename) return <h1 className="preview__title">{title}</h1>;

  return (
    <h1 className="preview__title">
      <input
        ref={field}
        className="preview__title-field"
        aria-label="Note name"
        value={draft}
        spellCheck={false}
        onChange={(event) => setDraft(event.target.value)}
        onFocus={() => setEditing(true)}
        onBlur={commit}
        onKeyDown={(event) => {
          if (event.key === "Enter") {
            event.preventDefault();
            field.current?.blur();
          } else if (event.key === "Escape") {
            event.preventDefault();
            abandoning.current = true;
            setDraft(title);
            field.current?.blur();
          }
        }}
      />
    </h1>
  );
}

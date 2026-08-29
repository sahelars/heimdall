/**
 * The note's title: the `# ` heading at the top of the file, which is also its
 * filename.
 *
 * There is no separate title to keep in step: a note is always called
 * something, and that something is both the name of the file and the heading it
 * opens with. Editing one changes the other, because they are the same fact
 * shown once.
 *
 * That is the conventional model, and it is why this is an editable heading rather
 * than a text field: it should read as the title of the note, not as a form.
 *
 * It is editable in source and inert in preview, which is the way round the two
 * modes already work: preview is for reading, and a rendered heading that
 * silently accepts typing is a heading nobody can tell from the rest of the
 * rendered note. The `#` shows only in source, for the same reason every other
 * marker does — and it sits outside the field, so it cannot be backspaced away.
 * Nor can the name: emptying it and leaving restores what was there, because a
 * note with no name is not a state the vault has.
 */

import { useEffect, useRef, useState, type ReactNode } from "react";

interface NoteTitleProps {
  /** The current name, without its extension. */
  title: string;
  /** Whether the heading can be typed into here — false in preview. */
  editable: boolean;
  /** Show the `# ` marker, as the source view shows every other marker. */
  marker?: boolean;
  className: string;
  /** Null when the note cannot be renamed from here. */
  onRename: ((name: string) => void) | null;
  /** Called after a commit made with Enter, to hand the caret to the body. */
  onLeave?: () => void;
}

export function NoteTitle({
  title,
  editable,
  marker,
  className,
  onRename,
  onLeave,
}: NoteTitleProps) {
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

  const heading = (children: ReactNode) => (
    <h1 className={className}>
      {marker ? (
        <span className={`${className}-marker`} aria-hidden="true">
          #
        </span>
      ) : null}
      {children}
    </h1>
  );

  if (!editable || !onRename) return heading(title);

  return heading(
    <input
      ref={field}
      className={`${className}-field`}
      aria-label="Note name"
      value={draft}
      spellCheck={false}
      onChange={(event) => setDraft(event.target.value)}
      onFocus={() => setEditing(true)}
      onBlur={commit}
      onKeyDown={(event) => {
        if (event.key === "Enter") {
          // A heading holds one line. Enter is "done with the name", so it
          // commits and drops into the body rather than doing nothing.
          event.preventDefault();
          field.current?.blur();
          onLeave?.();
        } else if (event.key === "Escape") {
          event.preventDefault();
          abandoning.current = true;
          setDraft(title);
          field.current?.blur();
        }
      }}
    />,
  );
}

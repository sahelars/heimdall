/**
 * Ask for one name.
 *
 * Not `window.prompt`: wry implements neither `runJavaScriptTextInputPanel` nor
 * the alert panel, so in the packaged application the browser dialogs return
 * null without showing anything — "New note" would quietly do nothing. This is
 * also the only version that can be styled to match, and the only one a test
 * can drive.
 */

import { useEffect, useRef, useState } from "react";

import { Button } from ".";

interface PromptProps {
  title: string;
  label: string;
  initial?: string;
  /** Shown under the field: a consequence worth knowing before confirming. */
  note?: string;
  open: boolean;
  onSubmit: (value: string) => void;
  onCancel: () => void;
}

export function Prompt({ title, label, initial = "", note, open, onSubmit, onCancel }: PromptProps) {
  const dialog = useRef<HTMLDialogElement | null>(null);
  const [value, setValue] = useState(initial);

  useEffect(() => {
    const element = dialog.current;
    if (!element) return;

    if (open && !element.open) {
      setValue(initial);
      if (typeof element.showModal === "function") element.showModal();
      else element.setAttribute("open", "");
    }
    if (!open && element.open) element.close();
  }, [open, initial]);

  return (
    <dialog
      ref={dialog}
      className="modal prompt"
      aria-label={title}
      onCancel={(event) => {
        event.preventDefault();
        onCancel();
      }}
      onClose={onCancel}
    >
      <form
        className="prompt__frame"
        onSubmit={(event) => {
          event.preventDefault();
          const trimmed = value.trim();
          if (trimmed) onSubmit(trimmed);
        }}
      >
        <label className="field__label" htmlFor="prompt-value">
          {label}
        </label>
        <input
          id="prompt-value"
          className="input"
          autoFocus
          value={value}
          onChange={(event) => setValue(event.target.value)}
        />
        {note ? <p className="field__hint">{note}</p> : null}
        <div className="row prompt__actions">
          <Button onClick={onCancel}>Cancel</Button>
          <Button type="submit" primary disabled={value.trim() === ""}>
            {title}
          </Button>
        </div>
      </form>
    </dialog>
  );
}

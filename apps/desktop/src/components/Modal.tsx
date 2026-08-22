/**
 * A modal dialog.
 *
 * Built on the native `<dialog>` element, which brings focus movement, a real
 * focus trap, Escape, top-layer stacking above the canvas, and focus restoration
 * on close — all of which would otherwise be hand-written and subtly wrong.
 */

import { useEffect, useRef, type ReactNode } from "react";

import { IconClose } from "./icons";

interface ModalProps {
  title: string;
  open: boolean;
  onClose: () => void;
  children: ReactNode;
}

export function Modal({ title, open, onClose, children }: ModalProps) {
  const dialog = useRef<HTMLDialogElement | null>(null);

  useEffect(() => {
    const element = dialog.current;
    if (!element) return;

    if (open && !element.open) {
      // `showModal` arrived in Safari 15.4 while the build targets 15, so the
      // fallback keeps the dialog usable rather than throwing on open.
      if (typeof element.showModal === "function") element.showModal();
      else element.setAttribute("open", "");

      // `showModal` focuses the first control it finds, which here is the close
      // button — so Settings opened with a focus ring drawn around its X, as if
      // something had been selected. Nothing in a settings sheet is *the*
      // control, so focus belongs on the sheet: Escape and Tab both still work
      // from there, and Tab reaches the close button first as before.
      element.focus();
    }
    if (!open && element.open) element.close();
  }, [open]);

  return (
    <dialog
      ref={dialog}
      className="modal"
      aria-label={title}
      // Focusable, but not in the tab order: the sheet is only ever focused by
      // the effect above, never by tabbing into it.
      tabIndex={-1}
      onCancel={(event) => {
        // Escape fires `cancel`; preventing the default keeps React the single
        // source of truth for whether the dialog is open.
        event.preventDefault();
        onClose();
      }}
      onClose={onClose}
      onClick={(event) => {
        // A click on the backdrop lands on the dialog element itself rather
        // than on any of its content.
        if (event.target === dialog.current) onClose();
      }}
    >
      <div className="modal__frame">
        <header className="modal__header">
          <h2>{title}</h2>
          <button
            type="button"
            className="icon-button"
            aria-label={`Close ${title.toLowerCase()}`}
            onClick={onClose}
          >
            <IconClose />
          </button>
        </header>
        {children}
      </div>
    </dialog>
  );
}

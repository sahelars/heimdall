/**
 * Ask before doing something that removes content.
 *
 * Deleting is recoverable — it moves into the vault's `.trash/` — but it is
 * still the sort of thing that should not happen from one stray click.
 */

import { useEffect, useRef, type ReactNode } from "react";

import { Button } from ".";

interface ConfirmProps {
  title: string;
  open: boolean;
  confirmLabel: string;
  onConfirm: () => void;
  onCancel: () => void;
  children: ReactNode;
}

export function Confirm({
  title,
  open,
  confirmLabel,
  onConfirm,
  onCancel,
  children,
}: ConfirmProps) {
  const dialog = useRef<HTMLDialogElement | null>(null);

  useEffect(() => {
    const element = dialog.current;
    if (!element) return;

    if (open && !element.open) {
      if (typeof element.showModal === "function") element.showModal();
      else element.setAttribute("open", "");
    }
    if (!open && element.open) element.close();
  }, [open]);

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
      <div className="confirm__frame">
        {children}
        <div className="row confirm__actions">
          <Button onClick={onCancel}>Cancel</Button>
          <Button primary onClick={onConfirm}>
            {confirmLabel}
          </Button>
        </div>
      </div>
    </dialog>
  );
}

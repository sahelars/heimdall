/**
 * Open a note by typing part of its name (⌘O).
 */

import { useEffect, useMemo, useRef, useState } from "react";

import { titleOf } from "../../api/source";
import { search } from "./fuzzy";

interface QuickSwitcherProps {
  open: boolean;
  paths: readonly string[];
  onOpen: (path: string) => void;
  onClose: () => void;
}

export function QuickSwitcher({ open, paths, onOpen, onClose }: QuickSwitcherProps) {
  const dialog = useRef<HTMLDialogElement | null>(null);
  const [query, setQuery] = useState("");
  const [selected, setSelected] = useState(0);

  const results = useMemo(() => search(query, paths, 50), [query, paths]);

  useEffect(() => {
    const element = dialog.current;
    if (!element) return;

    if (open && !element.open) {
      setQuery("");
      setSelected(0);
      if (typeof element.showModal === "function") element.showModal();
      else element.setAttribute("open", "");
    }
    if (!open && element.open) element.close();
  }, [open]);

  // A selection past the end of a narrowed list would leave Enter doing nothing.
  useEffect(() => setSelected(0), [query]);

  const choose = (index: number) => {
    const match = results[index];
    if (!match) return;
    onOpen(match.path);
    onClose();
  };

  return (
    <dialog
      ref={dialog}
      className="modal switcher"
      aria-label="Open note"
      onCancel={(event) => {
        event.preventDefault();
        onClose();
      }}
      onClose={onClose}
      onClick={(event) => {
        if (event.target === dialog.current) onClose();
      }}
    >
      <div className="switcher__frame">
        <input
          className="input switcher__input"
          aria-label="Find a note"
          placeholder="Find a note…"
          autoFocus
          value={query}
          onChange={(event) => setQuery(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "ArrowDown") {
              event.preventDefault();
              setSelected((previous) => Math.min(previous + 1, results.length - 1));
            } else if (event.key === "ArrowUp") {
              event.preventDefault();
              setSelected((previous) => Math.max(previous - 1, 0));
            } else if (event.key === "Enter") {
              event.preventDefault();
              choose(selected);
            }
          }}
        />

        <ul className="switcher__results" role="listbox" aria-label="Notes">
          {results.map((match, index) => (
            <li key={match.path}>
              <button
                type="button"
                role="option"
                aria-selected={index === selected}
                className={
                  index === selected ? "switcher__result switcher__result--selected" : "switcher__result"
                }
                onMouseEnter={() => setSelected(index)}
                onClick={() => choose(index)}
              >
                <span className="switcher__name">{titleOf(match.path)}</span>
                <span className="switcher__path">{match.path}</span>
              </button>
            </li>
          ))}
          {results.length === 0 && query !== "" ? (
            <li className="tree__note">No note matches “{query}”.</li>
          ) : null}
        </ul>
      </div>
    </dialog>
  );
}

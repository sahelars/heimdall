/**
 * A right-click menu.
 *
 * Positioned at the pointer and dismissed by anything else — a click elsewhere,
 * Escape, or scrolling the list it was opened from.
 */

import { useEffect, useRef, type ReactNode } from "react";

export interface MenuItem {
  label: string;
  onSelect: () => void;
  /** Set for anything that removes content, so it can read as weightier. */
  destructive?: boolean;
}

interface ContextMenuProps {
  x: number;
  y: number;
  items: MenuItem[];
  onClose: () => void;
  children?: ReactNode;
}

export function ContextMenu({ x, y, items, onClose }: ContextMenuProps) {
  const menu = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    const dismiss = (event: Event) => {
      if (menu.current?.contains(event.target as Node)) return;
      onClose();
    };
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };

    // Capture, so a click on a button elsewhere closes the menu before that
    // button's own handler runs and moves the ground under it.
    document.addEventListener("pointerdown", dismiss, true);
    document.addEventListener("keydown", onKey);
    window.addEventListener("blur", onClose);
    return () => {
      document.removeEventListener("pointerdown", dismiss, true);
      document.removeEventListener("keydown", onKey);
      window.removeEventListener("blur", onClose);
    };
  }, [onClose]);

  useEffect(() => {
    // Keep it on screen when opened near an edge.
    const element = menu.current;
    if (!element) return;
    const box = element.getBoundingClientRect();
    if (box.right > window.innerWidth) element.style.left = `${window.innerWidth - box.width - 8}px`;
    if (box.bottom > window.innerHeight) element.style.top = `${window.innerHeight - box.height - 8}px`;
  }, []);

  return (
    <div ref={menu} className="menu" role="menu" style={{ left: x, top: y }}>
      {items.map((item) => (
        <button
          key={item.label}
          type="button"
          role="menuitem"
          className={item.destructive ? "menu__item menu__item--destructive" : "menu__item"}
          onClick={() => {
            onClose();
            item.onSelect();
          }}
        >
          {item.label}
        </button>
      ))}
    </div>
  );
}

/**
 * One draggable pane divider.
 *
 * Pointer capture rather than window listeners: the pointer keeps reporting to
 * this element until it is released, so there is nothing to leak if the drag
 * ends outside the window.
 */

import { useRef } from "react";

import { clampPane } from "./panes";

interface DividerProps {
  side: "left" | "right";
  width: number;
  /**
   * What the pane on the other side of the note is taking.
   *
   * A divider's travel is bounded by the note's floor, and that floor depends
   * on both side panes — so this one cannot work out its own maximum alone.
   */
  opposite: number;
  label: string;
  onResize: (width: number) => void;
}

/** How far one arrow-key press moves a divider. */
const STEP = 16;

export function Divider({ side, width, opposite, label, onResize }: DividerProps) {
  const start = useRef<{ x: number; width: number } | null>(null);

  const container = (element: HTMLElement | null) =>
    element?.parentElement?.getBoundingClientRect().width ?? 0;

  return (
    <div
      className={`divider divider--${side}`}
      role="separator"
      aria-orientation="vertical"
      aria-label={label}
      aria-valuenow={width}
      tabIndex={0}
      onPointerDown={(event) => {
        // Primary button only: a right-click would otherwise arm a drag that
        // the matching pointerup never disarms.
        if (event.button !== 0) return;
        event.currentTarget.setPointerCapture(event.pointerId);
        start.current = { x: event.clientX, width };
      }}
      onPointerMove={(event) => {
        if (!start.current) return;
        const travelled = event.clientX - start.current.x;
        // The right pane grows as the pointer moves left, so its delta inverts.
        const delta = side === "left" ? travelled : -travelled;
        onResize(
          clampPane(start.current.width + delta, container(event.currentTarget), opposite),
        );
      }}
      onPointerUp={(event) => {
        event.currentTarget.releasePointerCapture(event.pointerId);
        start.current = null;
      }}
      // A cancelled drag — a system gesture, the window losing the pointer —
      // otherwise leaves the ref armed, and moving the mouse across the divider
      // with no button held would go on resizing the pane.
      onPointerCancel={() => {
        start.current = null;
      }}
      onLostPointerCapture={() => {
        start.current = null;
      }}
      onKeyDown={(event) => {
        // A divider nobody can reach without a mouse is a divider some people
        // cannot move at all.
        const direction = event.key === "ArrowLeft" ? -1 : event.key === "ArrowRight" ? 1 : 0;
        if (direction === 0) return;
        event.preventDefault();
        const delta = (side === "left" ? direction : -direction) * STEP;
        onResize(clampPane(width + delta, container(event.currentTarget), opposite));
      }}
    />
  );
}

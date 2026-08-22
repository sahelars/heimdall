/**
 * The three panes: tree, note, graph.
 *
 * A grid rather than nested flex, so the two dividers are real tracks and the
 * middle pane takes whatever is left without any width arithmetic.
 */

import { useEffect, useRef, type ReactNode } from "react";

import { Divider } from "./Divider";
import { fitPanes, paneTrack, type PaneWidths } from "./panes";

interface WorkspaceProps {
  widths: PaneWidths;
  onResize: (side: "left" | "right", width: number) => void;
  left: ReactNode;
  centre: ReactNode;
  right: ReactNode;
}

export function Workspace({ widths, onResize, left, centre, right }: WorkspaceProps) {
  const grid = useRef<HTMLDivElement | null>(null);

  // Give the note its room back when the window shrinks. Two panes dragged wide
  // in a large window would otherwise overflow the grid, scrolling the whole
  // application sideways and pushing the graph off the edge.
  useEffect(() => {
    const element = grid.current;
    if (!element) return;

    const observer = new ResizeObserver(([entry]) => {
      if (!entry) return;
      const fitted = fitPanes(widths, entry.contentRect.width);
      if (fitted.left !== widths.left) onResize("left", fitted.left);
      if (fitted.right !== widths.right) onResize("right", fitted.right);
    });
    observer.observe(element);
    return () => observer.disconnect();
  }, [widths, onResize]);

  return (
    <div
      className="workspace"
      ref={grid}
      style={{
        // Inline because these are dragged values, not design decisions.
        ["--pane-left" as string]: paneTrack(widths.left),
        ["--pane-right" as string]: paneTrack(widths.right),
      }}
    >
      <section className="workspace__pane workspace__pane--left" aria-label="Files">
        {widths.left > 0 ? left : null}
      </section>

      <Divider
        side="left"
        width={widths.left}
        label="Resize the file pane"
        onResize={(width) => onResize("left", width)}
      />

      <section className="workspace__pane" aria-label="Note">
        {centre}
      </section>

      <Divider
        side="right"
        width={widths.right}
        label="Resize the graph pane"
        onResize={(width) => onResize("right", width)}
      />

      <section className="workspace__pane" aria-label="Graph">
        {widths.right > 0 ? right : null}
      </section>
    </div>
  );
}

/**
 * A rendered mermaid diagram.
 *
 * Imported dynamically, so the library and its diagram chunks cost nothing until
 * a note actually contains a fence. It is by far the largest dependency here.
 *
 * `mermaid.render` returns SVG it has already sanitised, and `securityLevel:
 * "strict"` keeps its own DOMPurify pass on. This is the one place the
 * application inserts markup rather than building elements, and those two
 * together are what make it acceptable.
 */

import { useEffect, useState } from "react";

/**
 * A fresh id for every render attempt.
 *
 * `mermaid.render(id, …)` appends and later removes working elements keyed by
 * that id. React 19 runs an effect twice in development, and two overlapping
 * renders sharing one id delete each other's working node mid-flight — the
 * diagram comes out blank or throws. A counter cannot collide.
 */
let sequence = 0;

interface MermaidProps {
  code: string;
  /** Re-render when the theme changes: text metrics are baked into the SVG. */
  theme: string;
  /**
   * And when the accent changes, for the same reason.
   *
   * The value is not read here — the colours still come from the stylesheet
   * below — it is only the signal that the SVG mermaid already produced is out
   * of date.
   */
  accent: string | null;
}

export function Mermaid({ code, theme, accent }: MermaidProps) {
  const [svg, setSvg] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    setError(null);

    void (async () => {
      try {
        const { default: mermaid } = await import("mermaid");
        const style = getComputedStyle(document.body);
        const token = (name: string, fallback: string) =>
          style.getPropertyValue(name).trim() || fallback;

        // Mermaid's own palette is a pale lilac-on-cream, and it writes those
        // colours into the SVG itself — so a stylesheet rule loses to it
        // without `!important` on every selector. Handing it the vault's own
        // tokens is the supported way, and it keeps the values in CSS: nothing
        // here names a colour, it only passes one along.
        const bg = token("--bg", "black");
        const line = token("--link", "currentColor");
        mermaid.initialize({
          startOnLoad: false,
          theme: "base",
          securityLevel: "strict",
          fontFamily: style.fontFamily,
          themeVariables: {
            // The resolved theme is already known; sniffing it back out of a
            // colour value would put a literal in this file.
            darkMode: theme === "dark",
            background: bg,
            primaryColor: bg,
            primaryTextColor: line,
            primaryBorderColor: line,
            secondaryColor: bg,
            secondaryTextColor: line,
            secondaryBorderColor: line,
            tertiaryColor: bg,
            tertiaryTextColor: line,
            tertiaryBorderColor: line,
            mainBkg: bg,
            nodeBorder: line,
            nodeTextColor: line,
            lineColor: line,
            textColor: line,
            edgeLabelBackground: bg,
            clusterBkg: bg,
            clusterBorder: line,
            titleColor: line,
            // Sequence and class diagrams reach for these by different names.
            actorBkg: bg,
            actorBorder: line,
            actorTextColor: line,
            labelBoxBkgColor: bg,
            labelBoxBorderColor: line,
            labelTextColor: line,
            signalColor: line,
            signalTextColor: line,
            noteBkgColor: bg,
            noteTextColor: line,
            noteBorderColor: line,
          },
        });
        const rendered = await mermaid.render(`mermaid-${(sequence += 1)}`, code);
        if (!cancelled) setSvg(rendered.svg);
      } catch (thrown) {
        if (!cancelled) setError(thrown instanceof Error ? thrown.message : String(thrown));
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [code, theme, accent]);

  if (error !== null) {
    // A diagram that will not parse still has source worth reading, and saying
    // why beats an empty box.
    return (
      <div className="mermaid__error">
        <p className="muted">This diagram could not be drawn: {error}</p>
        <pre className="md-code">
          <code>{code}</code>
        </pre>
      </div>
    );
  }

  if (svg === null) {
    return (
      <pre className="md-code" aria-busy="true">
        <code>{code}</code>
      </pre>
    );
  }

  return (
    <div
      className="mermaid"
      role="img"
      aria-label="Diagram"
      dangerouslySetInnerHTML={{ __html: svg }}
    />
  );
}

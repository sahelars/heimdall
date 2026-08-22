/**
 * Rendering Markdown as React elements.
 *
 * `marked` is used as a lexer only — `marked.lexer()` returns tokens, and this
 * walks them into elements rather than letting the library build an HTML string.
 * That matters here more than it usually would: agents write into this vault, so
 * a note containing `<img src=x onerror=...>` is a realistic input rather than a
 * hypothetical one. Never producing HTML removes the whole class of problem, and
 * with it the need for a sanitiser.
 *
 * It also makes wikilinks real components wired to the workspace, instead of
 * markup that needs a click handler attached to it afterwards.
 */

import type { ReactNode } from "react";
import { lexer, type Token, type Tokens } from "marked";

import { splitWikilinks } from "./wikilinks";

export interface RenderContext {
  /** Where the note being rendered lives, for resolving relative links. */
  from: string;
  /** Follow a wikilink. */
  onOpenLink: (target: string, from: string) => void;
  /** Whether a target resolves to a note that exists. */
  resolves: (target: string, from: string) => boolean;
  /** Render a fenced mermaid block. */
  renderMermaid?: (code: string) => ReactNode;
}

export function parseMarkdown(source: string): Token[] {
  return lexer(source);
}

export function renderTokens(tokens: Token[], context: RenderContext): ReactNode[] {
  return tokens.map((token, index) => renderBlock(token, index, context));
}

function renderBlock(token: Token, key: number, context: RenderContext): ReactNode {
  switch (token.type) {
    case "heading": {
      const heading = token as Tokens.Heading;
      const Tag = `h${Math.min(heading.depth, 6)}` as "h1";
      return (
        <Tag key={key} className="md-heading">
          {renderInline(heading.tokens ?? [], context)}
        </Tag>
      );
    }

    case "paragraph": {
      const paragraph = token as Tokens.Paragraph;
      return (
        <p key={key} className="md-paragraph">
          {renderInline(paragraph.tokens ?? [], context)}
        </p>
      );
    }

    case "code": {
      const code = token as Tokens.Code;
      if (code.lang === "mermaid" && context.renderMermaid) {
        return <div key={key}>{context.renderMermaid(code.text)}</div>;
      }
      return (
        <pre key={key} className="md-code">
          <code>{code.text}</code>
        </pre>
      );
    }

    case "blockquote": {
      const quote = token as Tokens.Blockquote;
      return (
        <blockquote key={key} className="md-quote">
          {renderTokens(quote.tokens ?? [], context)}
        </blockquote>
      );
    }

    case "list": {
      const list = token as Tokens.List;
      const Tag = list.ordered ? "ol" : "ul";
      return (
        <Tag key={key} className="md-list" start={list.ordered ? Number(list.start) || 1 : undefined}>
          {list.items.map((item, itemIndex) => (
            <li key={itemIndex} className="md-list__item">
              {item.task ? (
                // Rendered, but not editable from the preview: a checkbox that
                // silently rewrote the file would be a write with no revision
                // behind it.
                <input type="checkbox" checked={item.checked ?? false} readOnly aria-label="Task" />
              ) : null}
              {renderTokens(item.tokens ?? [], context)}
            </li>
          ))}
        </Tag>
      );
    }

    case "table": {
      const table = token as Tokens.Table;
      return (
        <div key={key} className="md-table__scroll">
          <table className="md-table">
            <thead>
              <tr>
                {table.header.map((cell, cellIndex) => (
                  <th key={cellIndex}>{renderInline(cell.tokens ?? [], context)}</th>
                ))}
              </tr>
            </thead>
            <tbody>
              {table.rows.map((row, rowIndex) => (
                <tr key={rowIndex}>
                  {row.map((cell, cellIndex) => (
                    <td key={cellIndex}>{renderInline(cell.tokens ?? [], context)}</td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      );
    }

    case "hr":
      return <hr key={key} className="md-rule" />;

    case "space":
      return null;

    case "html":
      // Shown as the text it is. Rendering it would reintroduce exactly the
      // injection surface this renderer exists to avoid.
      return (
        <pre key={key} className="md-code">
          <code>{(token as Tokens.HTML).text}</code>
        </pre>
      );

    default: {
      const generic = token as { tokens?: Token[]; raw?: string };
      if (generic.tokens) {
        return (
          <p key={key} className="md-paragraph">
            {renderInline(generic.tokens, context)}
          </p>
        );
      }
      return generic.raw ? <p key={key} className="md-paragraph">{generic.raw}</p> : null;
    }
  }
}

function renderInline(tokens: Token[], context: RenderContext): ReactNode[] {
  return tokens.map((token, index) => {
    switch (token.type) {
      case "text": {
        const text = token as Tokens.Text;
        // A text token can carry nested inline tokens; when it does, they are
        // the truth and `text` is only their source.
        if (text.tokens && text.tokens.length > 0) {
          return <span key={index}>{renderInline(text.tokens, context)}</span>;
        }
        return <span key={index}>{withWikilinks(text.text, context)}</span>;
      }

      case "strong":
        return (
          <strong key={index}>{renderInline((token as Tokens.Strong).tokens ?? [], context)}</strong>
        );

      case "em":
        return <em key={index}>{renderInline((token as Tokens.Em).tokens ?? [], context)}</em>;

      case "del":
        return <del key={index}>{renderInline((token as Tokens.Del).tokens ?? [], context)}</del>;

      case "codespan":
        // Not scanned for wikilinks: `[[x]]` in backticks is a literal, which
        // is the same rule the Rust link scanner applies.
        return (
          <code key={index} className="md-codespan">
            {(token as Tokens.Codespan).text}
          </code>
        );

      case "br":
        return <br key={index} />;

      case "link": {
        const link = token as Tokens.Link;
        const external = /^[a-z][a-z0-9+.-]*:/i.test(link.href);
        if (external) {
          // No navigation: a note is untrusted input, and the WebView has
          // nowhere to send someone anyway.
          return (
            <span key={index} className="md-link md-link--external" title={link.href}>
              {renderInline(link.tokens ?? [], context)}
            </span>
          );
        }
        return (
          <LinkTo key={index} target={link.href} context={context}>
            {renderInline(link.tokens ?? [], context)}
          </LinkTo>
        );
      }

      case "escape":
        return <span key={index}>{(token as Tokens.Escape).text}</span>;

      case "image":
        // V1 exposes no attachment operation (SPEC §6), so there is nothing to
        // load. Showing the alt text beats showing a broken image.
        return (
          <span key={index} className="md-image">
            {(token as Tokens.Image).text || (token as Tokens.Image).href}
          </span>
        );

      default: {
        const generic = token as { tokens?: Token[]; raw?: string; text?: string };
        if (generic.tokens) return <span key={index}>{renderInline(generic.tokens, context)}</span>;
        return <span key={index}>{withWikilinks(generic.text ?? generic.raw ?? "", context)}</span>;
      }
    }
  });
}

/** Turn `[[target]]` runs inside plain text into links. */
function withWikilinks(text: string, context: RenderContext): ReactNode[] {
  return splitWikilinks(text).map((part, index) =>
    typeof part === "string" ? (
      <span key={index}>{part}</span>
    ) : (
      <LinkTo key={index} target={part.target} context={context}>
        {part.label}
      </LinkTo>
    ),
  );
}

export function LinkTo({
  target,
  context,
  children,
}: {
  target: string;
  context: RenderContext;
  children: ReactNode;
}) {
  const known = context.resolves(target, context.from);
  return (
    <button
      type="button"
      className={known ? "md-link" : "md-link md-link--unresolved"}
      title={known ? target : `${target} — no note with that name yet`}
      onClick={() => context.onOpenLink(target, context.from)}
    >
      {children}
    </button>
  );
}

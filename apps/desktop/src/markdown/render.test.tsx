/**
 * Rendering Markdown as elements (SPEC §17).
 *
 * The load-bearing property is that no HTML string is ever produced, so a note
 * an agent wrote cannot inject markup into the application.
 */

import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { parseMarkdown, renderTokens, type RenderContext } from "./render";

function draw(source: string, overrides: Partial<RenderContext> = {}) {
  const context: RenderContext = {
    from: "ideas/source.md",
    onOpenLink: vi.fn(),
    resolves: () => true,
    ...overrides,
  };
  const view = render(<div>{renderTokens(parseMarkdown(source), context)}</div>);
  return { ...view, context };
}

describe("blocks", () => {
  it("renders headings at their level", () => {
    draw("### Create Account Pipeline\n");
    expect(screen.getByRole("heading", { level: 3 })).toHaveTextContent("Create Account Pipeline");
  });

  it("renders fenced code as code rather than as markup", () => {
    draw("```\nconst x = 1;\n```\n");
    expect(screen.getByText("const x = 1;").tagName).toBe("CODE");
  });

  it("hands a mermaid fence to the diagram renderer", () => {
    const renderMermaid = vi.fn(() => <div data-testid="diagram" />);
    draw("```mermaid\ngraph TD\n  A --> B\n```\n", { renderMermaid });

    expect(renderMermaid).toHaveBeenCalledWith("graph TD\n  A --> B");
    expect(screen.getByTestId("diagram")).toBeInTheDocument();
  });

  it("renders lists and tables", () => {
    draw("- one\n- two\n");
    expect(screen.getAllByRole("listitem")).toHaveLength(2);

    draw("| a | b |\n| - | - |\n| 1 | 2 |\n");
    expect(screen.getByRole("table")).toBeInTheDocument();
  });
});

describe("safety", () => {
  it("shows raw HTML as text instead of rendering it", () => {
    // The whole reason this renderer walks tokens rather than emitting a
    // string: agents write into this vault.
    const { container } = draw('<img src="x" onerror="alert(1)">\n\n<b>bold?</b>\n');

    expect(container.querySelector("img")).toBeNull();
    expect(container.querySelector("b")).toBeNull();
    expect(container.textContent).toContain("<img");
  });

  it("does not turn an external link into something clickable", () => {
    const { container } = draw("[example](https://example.com)\n");
    expect(container.querySelector("a")).toBeNull();
    expect(screen.getByTitle("https://example.com")).toBeInTheDocument();
  });
});

describe("wikilinks", () => {
  it("turns a wikilink into a control that opens the note", async () => {
    const onOpenLink = vi.fn();
    draw("See [[profile]] for more.\n", { onOpenLink });

    await userEvent.click(screen.getByRole("button", { name: "profile" }));
    expect(onOpenLink).toHaveBeenCalledWith("profile", "ideas/source.md");
  });

  it("uses the alias as the label and the target for navigation", async () => {
    const onOpenLink = vi.fn();
    draw("See [[profile|my profile]].\n", { onOpenLink });

    await userEvent.click(screen.getByRole("button", { name: "my profile" }));
    expect(onOpenLink).toHaveBeenCalledWith("profile", "ideas/source.md");
  });

  it("marks a link with no note behind it rather than hiding the difference", () => {
    draw("See [[nowhere]].\n", { resolves: () => false });

    const link = screen.getByRole("button", { name: "nowhere" });
    expect(link.className).toContain("md-link--unresolved");
    expect(link).toHaveAttribute("title", expect.stringContaining("no note with that name yet"));
  });

  it("leaves a wikilink inside inline code alone", () => {
    // The same rule the Rust link scanner applies, so the preview and the graph
    // agree about what counts as a link.
    draw("write `[[literal]]` not a link\n");
    expect(screen.queryByRole("button", { name: "literal" })).toBeNull();
  });

  it("finds a wikilink inside emphasis", () => {
    draw("**bold [[profile]] here**\n");
    expect(screen.getByRole("button", { name: "profile" })).toBeInTheDocument();
  });
});

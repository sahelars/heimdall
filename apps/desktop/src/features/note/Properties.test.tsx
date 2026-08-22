/**
 * The properties table (SPEC §17).
 */

import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { Properties } from "./Properties";

const BLOCK = 'links:\n  - "[[profile]]"\n  - "[[articles]]"';

function show(frontmatter: string | null, editable = true, adding = false) {
  const onChange = vi.fn();
  const onOpenLink = vi.fn();
  const onAddingChange = vi.fn();
  render(
    <Properties
      frontmatter={frontmatter}
      onChange={editable ? onChange : null}
      onOpenLink={onOpenLink}
      adding={adding}
      onAddingChange={onAddingChange}
    />,
  );
  return { onChange, onOpenLink, onAddingChange };
}

describe("showing a block", () => {
  it("lists each property with its values", () => {
    show(BLOCK);
    expect(screen.getByText("links")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "profile" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "articles" })).toBeInTheDocument();
  });

  it("shows no table for a note with no --- block", () => {
    show(null);
    expect(screen.queryByRole("heading", { name: "Properties" })).toBeNull();
  });

  it("renders nothing at all for a note with no properties", () => {
    // An empty grid under a "Properties" heading is furniture, not
    // information. The way to add the first one is the note's own menu.
    for (const editable of [true, false]) {
      const { container, unmount } = render(
        <Properties
          frontmatter={null}
          onChange={editable ? vi.fn() : null}
          onOpenLink={vi.fn()}
          adding={false}
          onAddingChange={vi.fn()}
        />,
      );
      expect(container).toBeEmptyDOMElement();
      unmount();
    }
  });

  it("hides every control on a note that cannot be edited", () => {
    show(BLOCK, false);
    expect(screen.getByText("links")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Remove links" })).toBeNull();
  });

  it("opens the note a wikilink value points at", async () => {
    const { onOpenLink } = show(BLOCK);
    await userEvent.click(screen.getByRole("button", { name: "profile" }));
    expect(onOpenLink).toHaveBeenCalledWith("profile");
  });
});

describe("adding a property", () => {
  it("asks for a name and a value, not just a name", async () => {
    show(BLOCK, true, true);

    expect(screen.getByLabelText("Property")).toBeInTheDocument();
    expect(screen.getByLabelText("Value")).toBeInTheDocument();
  });

  it("writes both into the block", async () => {
    const { onChange } = show(BLOCK, true, true);
    await userEvent.type(screen.getByLabelText("Property"), "status");
    await userEvent.type(screen.getByLabelText("Value"), "draft");
    await userEvent.click(screen.getByRole("button", { name: "Add" }));

    expect(onChange).toHaveBeenCalledWith(expect.stringContaining("status: draft"));
    // And it leaves what was already there alone.
    expect(onChange.mock.calls[0]![0]).toContain('- "[[profile]]"');
  });

  it("appends to a list property rather than replacing it", async () => {
    const { onChange } = show(BLOCK, true, true);
    await userEvent.type(screen.getByLabelText("Property"), "links");
    await userEvent.type(screen.getByLabelText("Value"), "[[a third]]");
    await userEvent.click(screen.getByRole("button", { name: "Add" }));

    const written = onChange.mock.calls[0]![0] as string;
    expect(written).toContain("profile");
    expect(written).toContain("articles");
    expect(written).toContain("a third");
  });

  it("creates the block for a note that had none", async () => {
    const { onChange } = show(null, true, true);
    await userEvent.type(screen.getByLabelText("Property"), "status");
    await userEvent.type(screen.getByLabelText("Value"), "draft");
    await userEvent.click(screen.getByRole("button", { name: "Add" }));

    expect(onChange).toHaveBeenCalledWith(expect.stringContaining("status: draft"));
  });

  it("will not add a property with no name", async () => {
    show(BLOCK, true, true);
    expect(screen.getByRole("button", { name: "Add" })).toBeDisabled();
  });

  it("can be abandoned", async () => {
    const { onChange, onAddingChange } = show(BLOCK, true, true);
    await userEvent.type(screen.getByLabelText("Property"), "status");
    await userEvent.click(screen.getByRole("button", { name: "Cancel" }));

    expect(onChange).not.toHaveBeenCalled();
    expect(onAddingChange).toHaveBeenCalledWith(false);
  });
});

describe("removing", () => {
  it("drops one value and keeps the rest", async () => {
    const { onChange } = show(BLOCK);
    await userEvent.click(screen.getByRole("button", { name: "Remove articles" }));

    const written = onChange.mock.calls[0]![0] as string;
    expect(written).toContain("profile");
    expect(written).not.toContain("articles");
  });

  it("drops a whole property", async () => {
    const { onChange } = show(BLOCK);
    const row = screen.getByText("links").closest("dt")!;
    await userEvent.click(within(row).getByRole("button", { name: "Remove links" }));

    expect(onChange).toHaveBeenCalledWith("");
  });
});

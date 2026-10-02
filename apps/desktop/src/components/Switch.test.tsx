/**
 * The on/off control (SPEC §15).
 */

import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { Switch } from ".";

describe("Switch", () => {
  it("is a switch a screen reader can name and read the state of", async () => {
    const onChange = vi.fn();
    const { rerender } = render(
      <Switch id="s" label="Available to AI clients" checked={false} onChange={onChange} />,
    );

    const toggle = screen.getByRole("switch", { name: "Available to AI clients" });
    expect(toggle).toHaveAttribute("aria-checked", "false");
    expect(toggle).toHaveTextContent("Off");

    await userEvent.click(toggle);
    expect(onChange).toHaveBeenCalledWith(true);

    rerender(<Switch id="s" label="Available to AI clients" checked onChange={onChange} />);
    expect(toggle).toHaveAttribute("aria-checked", "true");
    expect(toggle).toHaveTextContent("On");
    expect(toggle).toHaveClass("switch--on");
  });

  it("answers the keyboard like any button, and nothing when disabled", async () => {
    const onChange = vi.fn();
    const { rerender } = render(<Switch id="s" label="L" checked={false} onChange={onChange} />);
    screen.getByRole("switch").focus();
    await userEvent.keyboard(" ");
    expect(onChange).toHaveBeenCalledWith(true);

    onChange.mockClear();
    rerender(<Switch id="s" label="L" checked={false} onChange={onChange} disabled />);
    await userEvent.click(screen.getByRole("switch"));
    expect(onChange).not.toHaveBeenCalled();
  });
});

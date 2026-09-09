//! The interface font may be proportional; the terminal font may not. The capability was always there
//! (fontStack passes a comma-bearing value through verbatim), but only the monospace faces were listed,
//! so nothing told the user a proportional face was allowed. These lock the two lists apart.
//!
//! The picker rides the shared Select since v0.1.104, so the group boundary is the rule Select draws for
//! `separatorBefore` rather than the heading rows the hand-rolled popup used to render. Select offers no
//! heading support and forking a shared component to add one buys a divider we already get.

import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { FontSelect } from "./SettingsModal";

afterEach(cleanup);

/** Open the picker. The trigger is a combobox, not a plain button, once Select renders it. */
function openOptions() {
  fireEvent.click(screen.getByRole("combobox"));
}

/** The rule Select draws above a `separatorBefore` row — how the two font groups are told apart now. */
function separatorAbove(name: string) {
  return (screen.getByRole("option", { name }) as HTMLElement).style.borderTop;
}

describe("FontSelect presets", () => {
  it("offers proportional faces when proportional is set", () => {
    render(<FontSelect value={null} onChange={() => {}} label="Interface font" proportional />);
    openOptions();
    expect(screen.getByRole("option", { name: "Inter" })).toBeTruthy();
    expect(screen.getByRole("option", { name: "Helvetica Neue" })).toBeTruthy();
    // The monospace faces stay available for a user who wants a mono interface, below a rule.
    expect(screen.getByRole("option", { name: "JetBrains Mono" })).toBeTruthy();
    expect(separatorAbove("JetBrains Mono")).toBeTruthy();
  });

  it("offers monospace faces only by default (the terminal call site)", () => {
    render(<FontSelect value={null} onChange={() => {}} label="Terminal font" />);
    openOptions();
    expect(screen.getByRole("option", { name: "JetBrains Mono" })).toBeTruthy();
    expect(screen.queryByRole("option", { name: "Inter" })).toBeNull();
    expect(screen.queryByRole("option", { name: "Helvetica Neue" })).toBeNull();
    // With one group there is nothing to divide, so the rule must not appear.
    expect(separatorAbove("JetBrains Mono")).toBeFalsy();
  });

  it("stores a full CSS stack and shows the short label for it", () => {
    const onChange = vi.fn();
    const { rerender } = render(
      <FontSelect value={null} onChange={onChange} label="Interface font" proportional />,
    );
    openOptions();
    fireEvent.click(screen.getByRole("option", { name: "Inter" }));
    expect(onChange).toHaveBeenCalledWith("Inter, system-ui, sans-serif");

    rerender(
      <FontSelect
        value="Inter, system-ui, sans-serif"
        onChange={onChange}
        label="Interface font"
        proportional
      />,
    );
    expect(screen.getByRole("combobox").textContent).toContain("Inter");
    // A preset must not read back as a custom entry: the stack is a listed option, so it is that row
    // that reads as selected and the free-text row that does not.
    openOptions();
    expect(screen.getByRole("option", { name: "Inter" }).getAttribute("aria-selected")).toBe("true");
    expect(screen.getByRole("option", { name: "Custom…" }).getAttribute("aria-selected")).toBe(
      "false",
    );
  });

  it("still accepts a free-text family", () => {
    const onChange = vi.fn();
    render(<FontSelect value={null} onChange={onChange} label="Interface font" proportional />);
    openOptions();
    fireEvent.click(screen.getByRole("option", { name: "Custom…" }));
    const input = screen.getByPlaceholderText("Fira Code");
    fireEvent.change(input, { target: { value: "Comic Sans MS" } });
    fireEvent.keyDown(input, { key: "Enter" });
    expect(onChange).toHaveBeenCalledWith("Comic Sans MS");
  });
});

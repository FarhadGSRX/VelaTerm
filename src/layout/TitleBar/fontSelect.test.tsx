//! The interface font may be proportional; the terminal font may not. The capability was always there
//! (fontStack passes a comma-bearing value through verbatim), but only the monospace faces were listed,
//! so nothing told the user a proportional face was allowed. These lock the two lists apart.

import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { FontSelect } from "./SettingsModal";

afterEach(cleanup);

/** Open the picker and return the labels of every row in the dropdown. */
function openOptions() {
  fireEvent.click(screen.getByRole("button"));
  return Array.from(document.querySelectorAll("div")).map((d) => d.textContent);
}

describe("FontSelect presets", () => {
  it("offers proportional faces when proportional is set", () => {
    render(<FontSelect value={null} onChange={() => {}} proportional />);
    openOptions();
    expect(screen.getByText("Inter")).toBeTruthy();
    expect(screen.getByText("Helvetica Neue")).toBeTruthy();
    expect(screen.getByText("Proportional")).toBeTruthy();
    // The monospace faces stay available for a user who wants a mono interface.
    expect(screen.getByText("JetBrains Mono")).toBeTruthy();
  });

  it("offers monospace faces only by default (the terminal call site)", () => {
    render(<FontSelect value={null} onChange={() => {}} />);
    openOptions();
    expect(screen.getByText("JetBrains Mono")).toBeTruthy();
    expect(screen.queryByText("Inter")).toBeNull();
    expect(screen.queryByText("Helvetica Neue")).toBeNull();
    expect(screen.queryByText("Proportional")).toBeNull();
  });

  it("stores a full CSS stack and shows the short label for it", () => {
    const onChange = vi.fn();
    const { rerender } = render(<FontSelect value={null} onChange={onChange} proportional />);
    openOptions();
    fireEvent.click(screen.getByText("Inter"));
    expect(onChange).toHaveBeenCalledWith("Inter, system-ui, sans-serif");

    rerender(<FontSelect value="Inter, system-ui, sans-serif" onChange={onChange} proportional />);
    expect(screen.getByRole("button").textContent).toContain("Inter");
    // A preset must not read back as a custom entry.
    openOptions();
    expect(screen.getByText("Custom…").parentElement?.querySelector("svg")).toBeNull();
  });

  it("still accepts a free-text family", () => {
    const onChange = vi.fn();
    render(<FontSelect value={null} onChange={onChange} proportional />);
    openOptions();
    fireEvent.click(screen.getByText("Custom…"));
    const input = screen.getByPlaceholderText("Fira Code");
    fireEvent.change(input, { target: { value: "Comic Sans MS" } });
    fireEvent.keyDown(input, { key: "Enter" });
    expect(onChange).toHaveBeenCalledWith("Comic Sans MS");
  });
});

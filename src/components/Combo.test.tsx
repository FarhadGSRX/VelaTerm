//! The combo is the control a launch field uses: a dropdown that also takes typed text. These cases pin
//! the three commitments the launch dialogs rely on — the committed value reads as its option label, the
//! list filters while typing, and text commits either as a custom value or not at all.
import { fireEvent, render, screen } from "@testing-library/react";
import { useState } from "react";
import { describe, expect, it, vi } from "vitest";
import Combo from "./Combo";

const OPTIONS = [
  { value: "claude", label: "Claude" },
  { value: "codex", label: "Codex" },
];

function Harness({ allowCustom = true, onCommit }: { allowCustom?: boolean; onCommit?: (v: string) => void }) {
  const [value, setValue] = useState("claude");
  return (
    <Combo
      value={value}
      onChange={setValue}
      onCommit={onCommit}
      allowCustom={allowCustom}
      options={OPTIONS}
      ariaLabel="agent"
    />
  );
}

describe("Combo", () => {
  it("shows the committed value under its option label", () => {
    render(<Harness />);
    expect((screen.getByRole("combobox", { name: "agent" }) as HTMLInputElement).value).toBe("Claude");
  });

  it("filters the list to the typed text and commits the highlighted row", () => {
    render(<Harness />);
    const input = screen.getByRole("combobox");
    fireEvent.click(input);
    fireEvent.change(input, { target: { value: "cod" } });
    expect(screen.queryByRole("option", { name: "Claude" })).toBeNull();
    fireEvent.keyDown(input, { key: "ArrowDown" });
    fireEvent.keyDown(input, { key: "Enter" });
    expect((input as HTMLInputElement).value).toBe("Codex");
    expect(screen.queryByRole("listbox")).toBeNull();
  });

  it("commits typed text that matches no option and reports it as a choice", () => {
    const onCommit = vi.fn();
    render(<Harness onCommit={onCommit} />);
    const input = screen.getByRole("combobox");
    fireEvent.change(input, { target: { value: "gpt-5.5" } });
    fireEvent.keyDown(input, { key: "Enter" });
    expect((input as HTMLInputElement).value).toBe("gpt-5.5");
    expect(onCommit).toHaveBeenCalledWith("gpt-5.5");
  });

  it("commits a value when the field loses focus", () => {
    render(<Harness />);
    const input = screen.getByRole("combobox");
    fireEvent.change(input, { target: { value: "sonnet" } });
    fireEvent.blur(input);
    expect((input as HTMLInputElement).value).toBe("sonnet");
  });

  it("restores the committed value when custom text is not accepted", () => {
    const onChange = vi.fn();
    render(<Combo value="claude" onChange={onChange} options={OPTIONS} ariaLabel="agent" />);
    const input = screen.getByRole("combobox");
    fireEvent.change(input, { target: { value: "nope" } });
    fireEvent.keyDown(input, { key: "Enter" });
    expect(onChange).not.toHaveBeenCalled();
    expect((input as HTMLInputElement).value).toBe("Claude");
  });

  it("commits a clicked row by its value", () => {
    const onCommit = vi.fn();
    render(<Harness onCommit={onCommit} />);
    fireEvent.click(screen.getByRole("combobox"));
    fireEvent.click(screen.getByRole("option", { name: "Codex" }));
    expect(onCommit).toHaveBeenCalledWith("codex");
  });
});

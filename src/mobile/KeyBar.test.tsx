import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
const write = vi.hoisted(() => vi.fn());
vi.mock("../i18n", () => ({ useT: () => (key: string) => key }));
vi.mock("../ipc/commands", () => ({ ptyWrite: write }));
vi.mock("../terminal/registry", () => ({ isAppCursorMode: () => false }));
import { KeyBar } from "./KeyBar";

afterEach(() => { cleanup(); vi.resetAllMocks(); });

const labels = () => screen.getAllByRole("button").map((b) => b.textContent);
const mount = (agent: boolean) => render(<KeyBar sessionId="s1" agent={agent} composing={false}
  onCompose={() => {}} onKeyboard={() => {}} onImages={() => {}} />);

describe("mobile key bar", () => {
  it("keeps only the keys an agent needs", () => {
    write.mockResolvedValue(undefined);
    mount(true);
    expect(labels()).toEqual(["Esc", "⇧Tab", "↑", "↓", "⏎", "^C", "📷"]);
    fireEvent.click(screen.getByText("⇧Tab"));
    expect(write).toHaveBeenCalledWith("s1", "\x1b[Z");
  });

  it("keeps the full set in a shell", () => {
    mount(false);
    expect(labels()).toEqual(["✎", "⌨", "Esc", "Tab", "⇧Tab", "Ctrl", "↑", "↓", "←", "→", "⏎", "^C", "📷"]);
  });
});

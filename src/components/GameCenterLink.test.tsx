import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";

const state = vi.hoisted(() => ({ locale: "en", desktop: false, electron: false, open: vi.fn() }));
vi.mock("../i18n", () => ({ getLocale: () => state.locale, useT: () => () => "Game Center" }));
vi.mock("../ipc/transport", () => ({ get isTauri() { return state.desktop; } }));
vi.mock("../platform", () => ({ env: { get isElectron() { return state.electron; } } }));
vi.mock("../store/termStore", () => ({ useTermStore: { getState: () => ({ openBrowserTab: state.open }) } }));
import { GameCenterLink } from "./GameCenterLink";

afterEach(() => { cleanup(); state.locale = "en"; state.desktop = false; state.electron = false; state.open.mockClear(); });

describe("Game Center entry", () => {
  it("provides a real website link for browser clients", () => {
    render(<GameCenterLink />);
    expect(screen.getByRole("link").getAttribute("href")).toBe("https://velaterm.com/game-center");
    expect(screen.getByRole("link").getAttribute("target")).toBe("_blank");
  });
  it.each(["desktop", "electron"] as const)("opens the localized center in the %s browser pane", (shell) => {
    state[shell] = true;
    state.locale = "zh-CN";
    render(<GameCenterLink />);
    fireEvent.click(screen.getByRole("link"));
    expect(state.open).toHaveBeenCalledWith("https://velaterm.com/zh-CN/game-center", { chromeHidden: true });
  });
  it("preserves modified clicks for standard link behavior", () => {
    state.desktop = true;
    render(<GameCenterLink />);
    fireEvent.click(screen.getByRole("link"), { ctrlKey: true });
    fireEvent.click(screen.getByRole("link"), { metaKey: true });
    expect(state.open).not.toHaveBeenCalled();
  });
});

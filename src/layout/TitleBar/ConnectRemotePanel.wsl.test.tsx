import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";

vi.mock("../../platform/env", () => ({ env: { isTauri: true, isElectron: false } }));
vi.mock("../../ipc/transport", () => ({ invoke: vi.fn(), listen: vi.fn().mockResolvedValue(() => {}) }));
vi.mock("../../hooks/nativeViewSuspend", () => ({ useSuspendNativeViews: vi.fn() }));
vi.mock("../../sharing/RemoteDevices", () => ({ RemoteDevices: () => null }));

import { invoke } from "../../ipc/transport";
import { setLang } from "../../i18n";
import { ConnectRemotePanel } from "./ConnectRemotePanel";

const catalog = { supported: true, distributions: ["Ubuntu", "Debian"], selected: "Ubuntu", error: null };

beforeEach(() => {
  setLang("en");
  vi.clearAllMocks();
  history.replaceState(null, "", "/?connect=wsl");
  vi.mocked(invoke).mockImplementation(async (command) => command === "wsl_options" ? catalog : []);
});
afterEach(cleanup);

it("restores the distribution from its URL and connects to that exact backend environment", async () => {
  history.replaceState(null, "", "/?connect=wsl&wslDistribution=Debian");
  const close = vi.fn();
  render(<ConnectRemotePanel onClose={close} />);
  expect(await screen.findByRole("combobox", { name: "Linux distribution" })).toBeTruthy();
  expect(screen.getByRole("combobox").textContent).toContain("Debian");
  fireEvent.click(screen.getByRole("button", { name: "Connect" }));
  await waitFor(() => expect(invoke).toHaveBeenCalledWith("wsl_connect", { distribution: "Debian", restartExisting: false, connectId: expect.any(String) }));
  await waitFor(() => expect(close).toHaveBeenCalledOnce());
});

it("writes selection to the URL and restores the panel when navigation returns to WSL", async () => {
  render(<ConnectRemotePanel onClose={() => {}} />);
  fireEvent.click(await screen.findByRole("combobox", { name: "Linux distribution" }));
  fireEvent.click(screen.getByRole("option", { name: "Debian" }));
  expect(new URLSearchParams(location.search).get("wslDistribution")).toBe("Debian");
  const wslUrl = screen.getByRole("link", { name: "WSL" }).getAttribute("href")!;
  fireEvent.click(screen.getByRole("link", { name: "SSH" }));
  expect(new URLSearchParams(location.search).get("connect")).toBe("ssh");
  await act(async () => { history.replaceState(null, "", wslUrl); window.dispatchEvent(new PopStateEvent("popstate")); });
  expect(screen.getByRole("combobox").textContent).toContain("Debian");
});

it("blocks stale selections and does not fall back to another distribution", async () => {
  history.replaceState(null, "", "/?connect=wsl&wslDistribution=Removed");
  render(<ConnectRemotePanel onClose={() => {}} />);
  expect(await screen.findByText("This distribution is no longer available. Select another one.")).toBeTruthy();
  expect((screen.getByRole("button", { name: "Connect" }) as HTMLButtonElement).disabled).toBe(true);
  expect(invoke).not.toHaveBeenCalledWith("wsl_connect", expect.anything());
});

it("shows discovery failures and refreshes without using an old catalog", async () => {
  vi.mocked(invoke).mockImplementation(async command => command === "wsl_options" ? { ...catalog, distributions: [], selected: null, error: "WSL could not start" } : []);
  render(<ConnectRemotePanel onClose={() => {}} />);
  expect(await screen.findByRole("alert")).toHaveProperty("textContent", "WSL could not start");
  expect((screen.getByRole("button", { name: "Connect" }) as HTMLButtonElement).disabled).toBe(true);
  vi.mocked(invoke).mockImplementation(async command => command === "wsl_options" ? catalog : []);
  fireEvent.click(screen.getByRole("button", { name: "Refresh" }));
  expect(await screen.findByRole("combobox")).toBeTruthy();
  expect(screen.queryByRole("alert")).toBeNull();
});

it("renders an empty installation state and permits retry after a connection failure", async () => {
  vi.mocked(invoke).mockImplementation(async command => command === "wsl_options" ? { ...catalog, distributions: [], selected: null } : []);
  render(<ConnectRemotePanel onClose={() => {}} />);
  expect(await screen.findByText(/No WSL distributions found/)).toBeTruthy();
  vi.mocked(invoke).mockImplementation(async command => {
    if (command === "wsl_options") return catalog;
    if (command === "wsl_connect") throw new Error("Server unavailable");
    return [];
  });
  fireEvent.click(screen.getByRole("button", { name: "Refresh" }));
  await screen.findByRole("combobox");
  fireEvent.click(screen.getByRole("button", { name: "Connect" }));
  expect(await screen.findByText("Error: Server unavailable")).toBeTruthy();
  expect((screen.getByRole("button", { name: "Connect" }) as HTMLButtonElement).disabled).toBe(false);
});

it("requires explicit upgrade confirmation and never transfers it to another distribution", async () => {
  vi.mocked(invoke).mockImplementation(async command => {
    if (command === "wsl_options") return catalog;
    if (command === "wsl_connect") throw new Error("wsl_version_running");
    return [];
  });
  render(<ConnectRemotePanel onClose={() => {}} />);
  await screen.findByRole("combobox");
  fireEvent.click(screen.getByRole("button", { name: "Connect" }));
  fireEvent.click(await screen.findByRole("button", { name: "Restart server and connect" }));
  await waitFor(() => expect(invoke).toHaveBeenCalledWith("wsl_connect", { distribution: "Ubuntu", restartExisting: true, connectId: expect.any(String) }));
  await act(async () => { history.replaceState(null, "", "/?connect=wsl&wslDistribution=Debian"); window.dispatchEvent(new PopStateEvent("popstate")); });
  fireEvent.click(screen.getByRole("button", { name: "Connect" }));
  await waitFor(() => expect(invoke).toHaveBeenCalledWith("wsl_connect", { distribution: "Debian", restartExisting: false, connectId: expect.any(String) }));
});

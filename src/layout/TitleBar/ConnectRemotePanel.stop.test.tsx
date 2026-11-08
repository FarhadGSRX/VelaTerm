import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";

vi.mock("../../platform/env", () => ({ env: { isTauri: true, isElectron: false } }));
vi.mock("../../ipc/transport", () => ({ invoke: vi.fn(), listen: vi.fn().mockResolvedValue(() => {}) }));
vi.mock("../../hooks/nativeViewSuspend", () => ({ useSuspendNativeViews: vi.fn() }));
vi.mock("../../sharing/RemoteDevices", () => ({ RemoteDevices: () => null }));

import { invoke } from "../../ipc/transport";
import { setLang } from "../../i18n";
import { ConnectRemotePanel } from "./ConnectRemotePanel";

// The pending ssh_connect call; the test settles it the way the backend would after a cancel.
let settle: { reject: (e: unknown) => void } | null = null;

beforeEach(() => {
  setLang("en");
  vi.clearAllMocks();
  settle = null;
  history.replaceState(null, "", "/?connect=ssh");
  vi.mocked(invoke).mockImplementation(async (command) => {
    if (command === "wsl_options") return { supported: false, distributions: [], selected: null, error: null };
    if (command === "ssh_probe_host") return { status: "known", target: "me@host", keyType: "ed25519", fingerprint: "SHA256:x" };
    if (command === "ssh_connect") return new Promise((_, reject) => { settle = { reject }; });
    return [];
  });
});
afterEach(cleanup);

it("stops a running SSH connect, cancels it in the backend, and ignores its late result", async () => {
  const close = vi.fn();
  render(<ConnectRemotePanel onClose={close} />);
  fireEvent.change(screen.getByPlaceholderText("user@host[:port]"), { target: { value: "me@host" } });
  fireEvent.click(screen.getByRole("button", { name: "Connect" }));
  await waitFor(() => expect(settle).not.toBeNull());
  const args = vi.mocked(invoke).mock.calls.find(([c]) => c === "ssh_connect")![1] as { connectId: string };
  expect(args.connectId).toBeTruthy();

  fireEvent.click(screen.getByRole("button", { name: "Stop" }));
  expect(invoke).toHaveBeenCalledWith("connect_cancel", { connectId: args.connectId });
  expect((screen.getByRole("button", { name: "Connect" }) as HTMLButtonElement).disabled).toBe(false);
  expect(screen.queryByRole("button", { name: "Stop" })).toBeNull();

  settle!.reject("__VLX_CONNECT_CANCELLED__");
  await new Promise((r) => setTimeout(r, 0));
  expect(close).not.toHaveBeenCalled();
  expect(screen.queryByText(/CANCELLED/)).toBeNull();
});

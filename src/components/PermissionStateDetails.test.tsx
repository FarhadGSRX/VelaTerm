import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { useSessionPermissionState, type SessionPermissionState } from "../hooks/useSessionPermissionState";
import { currentPermissionLabel, PermissionStateDetails } from "./PermissionStateDetails";
import { invoke, listen, onTransportDisconnect, onTransportReconnect } from "../ipc/transport";

vi.mock("../ipc/transport", async original => ({ ...await original<typeof import("../ipc/transport")>(),
  invoke: vi.fn(), listen: vi.fn(), onTransportDisconnect: vi.fn(), onTransportReconnect: vi.fn() }));
let events: Map<string, (event: unknown) => void>;
let disconnect: () => void;
let reconnect: () => void;
const pending: SessionPermissionState = { configured: "full-access", current: "auto", launch: null,
  pending: "full-access", activation: "nextTurn", running: true };

function View({ id = "s" }: { id?: string }) {
  const state = useSessionPermissionState(id);
  return <><strong>{currentPermissionLabel(state?.value)}</strong><PermissionStateDetails state={state?.value} error={state?.error} /></>;
}

beforeEach(() => {
  vi.resetAllMocks();
  events = new Map();
  vi.mocked(listen).mockImplementation((name, callback) => {
    events.set(name, callback);
    return Promise.resolve(() => { events.delete(name); });
  });
  vi.mocked(onTransportDisconnect).mockImplementation(callback => { disconnect = callback; return () => {}; });
  vi.mocked(onTransportReconnect).mockImplementation(callback => { reconnect = callback; return () => {}; });
  vi.mocked(invoke).mockResolvedValue(pending);
});
afterEach(cleanup);

it("shows the saved permission without a launch notice when the session is stopped", async () => {
  vi.mocked(invoke).mockResolvedValue({ configured: "full-access", current: null, launch: null,
    pending: "full-access", activation: "nextStart", running: false });
  render(<View />);
  expect(await screen.findByText("Full Access")).toBeTruthy();
  expect(screen.queryByText("Not running")).toBeNull();
  expect(screen.queryByRole("status")).toBeNull();
});

it("shows acknowledged Codex permissions separately from the next turn's saved choice", async () => {
  render(<View />);
  expect(await screen.findByText("Auto mode")).toBeTruthy();
  expect(screen.getByText("Applies to the next message: Full Access")).toBeTruthy();
  vi.mocked(invoke).mockResolvedValue({ ...pending, current: "full-access", pending: null, activation: "applied" });
  act(() => events.get("chat://event/s")!({ type: "settingsChanged" }));
  expect(await screen.findByText("Full Access")).toBeTruthy();
  expect(screen.queryByText("Applied")).toBeNull();
  expect(screen.queryByRole("status")).toBeNull();
  expect(screen.queryByText("Applies to the next message: Full Access")).toBeNull();
});

it("treats the launch mode as the running permission without a separate launch notice", async () => {
  vi.mocked(invoke).mockResolvedValue({ configured: "bypassPermissions", current: null, launch: "default",
    pending: "bypassPermissions", activation: "restart", running: true });
  render(<View />);
  expect(await screen.findByText("Always Ask")).toBeTruthy();
  expect(screen.getByText("Applies after restarting this session: Bypass")).toBeTruthy();
  expect(screen.queryByText("Current permissions unconfirmed")).toBeNull();
  vi.mocked(invoke).mockResolvedValue({ configured: "bypassPermissions", current: null, launch: "bypassPermissions",
    pending: null, activation: "unconfirmed", running: true });
  act(() => events.get("pty://status/s")!({ kind: "agent", agent: "claude" }));
  expect(await screen.findByText("Bypass")).toBeTruthy();
  expect(screen.queryByText("Applies after restarting this session: Bypass")).toBeNull();
  expect(screen.queryByRole("status")).toBeNull();
});

it("reports unconfirmed permissions only when neither a confirmation nor a launch mode exists", async () => {
  vi.mocked(invoke).mockResolvedValue({ configured: "auto", current: null, launch: null,
    pending: null, activation: "unconfirmed", running: true });
  render(<View />);
  expect(await screen.findAllByText("Current permissions unconfirmed")).toHaveLength(2);
});

it("discards stale evidence on disconnect and reloads authoritative state on reconnect", async () => {
  render(<View />);
  await screen.findByText("Auto mode");
  act(() => disconnect());
  expect(screen.queryByText("Auto mode")).toBeNull();
  expect(screen.queryByText("Applies to the next message: Full Access")).toBeNull();
  vi.mocked(invoke).mockResolvedValue({ ...pending, current: "full-access", pending: null, activation: "applied" });
  act(() => reconnect());
  expect(await screen.findByText("Full Access")).toBeTruthy();
});

it("does not let a late response replace another session or restore state after a failed refresh", async () => {
  let finish!: (state: SessionPermissionState) => void;
  vi.mocked(invoke).mockImplementationOnce(() => new Promise<SessionPermissionState>(resolve => { finish = resolve; }) as Promise<never>);
  const view = render(<View />);
  view.rerender(<View id="other" />);
  await screen.findByText("Auto mode");
  await act(async () => finish({ ...pending, current: "read-only" }));
  expect(screen.queryByText("Read-only")).toBeNull();
  vi.mocked(invoke).mockRejectedValue(new Error("Offline"));
  act(() => reconnect());
  await waitFor(() => expect(screen.queryByText("Auto mode")).toBeNull());
  expect(screen.queryByText("Applied")).toBeNull();
});

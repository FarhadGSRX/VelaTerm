import { renderHook } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import type { Session } from "../../../types";
import { useAgentHandoff } from "./agentHandoff";

const mocks = vi.hoisted(() => ({
  state: { runtimes: {} as Record<string, { status: string }>, openSession: vi.fn() },
  create: vi.fn((_session: string, text: string) => ({ id: "msg-1", text })),
  deliver: vi.fn(async () => "sent"),
  start: vi.fn(async () => {}),
  write: vi.fn(async () => {}),
}));
vi.mock("../../../store/termStore", () => {
  const useTermStore = (select: (s: unknown) => unknown) => select(mocks.state);
  useTermStore.getState = () => mocks.state;
  return { useTermStore };
});
vi.mock("../../sessionMenuShared", () => ({ kindLabel: (kind: string) => kind.toUpperCase() }));
vi.mock("../../CenterPane/session/outbox", () => ({ createSubmission: mocks.create, deliverSubmission: mocks.deliver }));
vi.mock("../../../ipc/chat", () => ({ chatStart: mocks.start }));
vi.mock("../../../ipc/commands", () => ({ ptyWrite: mocks.write }));

const session = (extra: Partial<Session>) => ({ id: "s1", kind: "claude", engine: "chat", ...extra }) as Session;

beforeEach(() => {
  vi.clearAllMocks();
  mocks.state.runtimes = {};
});

it("has no handoff for terminals, browsers, or a missing session", () => {
  expect(renderHook(() => useAgentHandoff(undefined)).result.current).toBeNull();
  expect(renderHook(() => useAgentHandoff(session({ kind: "terminal" }))).result.current).toBeNull();
  expect(renderHook(() => useAgentHandoff(session({ kind: "browser" }))).result.current).toBeNull();
});

it("sends to a conversation through the outbox, starting the agent if needed", async () => {
  const handoff = renderHook(() => useAgentHandoff(session({}))).result.current!;
  expect(handoff).toMatchObject({ label: "CLAUDE", sends: true });
  await handoff.send("Resolve it");
  expect(mocks.state.openSession).toHaveBeenCalledWith("s1");
  expect(mocks.create).toHaveBeenCalledWith("s1", "Resolve it", [], "queue");
  expect(mocks.deliver).toHaveBeenCalledTimes(1);
  const start = (mocks.deliver.mock.calls[0] as unknown as [string, unknown, () => Promise<void>])[2];
  await start();
  expect(mocks.start).toHaveBeenCalledWith("s1");
});

it("pastes into a running terminal agent without pressing Enter", async () => {
  const tui = session({ engine: "tui" });
  expect(renderHook(() => useAgentHandoff(tui)).result.current).toBeNull();
  mocks.state.runtimes = { s1: { status: "running" } };
  const handoff = renderHook(() => useAgentHandoff(tui)).result.current!;
  expect(handoff.sends).toBe(false);
  await handoff.send("Explain");
  expect(mocks.write).toHaveBeenCalledWith("s1", "\x1b[200~Explain\x1b[201~");
});

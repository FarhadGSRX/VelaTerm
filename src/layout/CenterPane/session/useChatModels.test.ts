import { act, cleanup, renderHook, waitFor } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { chatModels, type ChatModel } from "../../../ipc/chat";
import { onTransportReconnect } from "../../../ipc/transport";
import { useChatModels } from "./useChatModels";

vi.mock("../../../ipc/chat", () => ({ chatModels: vi.fn() }));
vi.mock("../../../ipc/transport", () => ({ onTransportReconnect: vi.fn(() => () => {}) }));
afterEach(() => { cleanup(); vi.useRealTimers(); vi.clearAllMocks(); });

const models: ChatModel[] = [{ id: "reported", label: "Reported", description: "", effortLevels: [], curated: true, largeContext: false }];

it("ignores a discovery response from a previous session", async () => {
  let finish!: (value: ChatModel[]) => void;
  vi.mocked(chatModels).mockImplementation(id => id === "old"
    ? new Promise(resolve => { finish = resolve; }) : Promise.resolve(models));
  const { result, rerender } = renderHook(({ id }) => useChatModels(id, 0), { initialProps: { id: "old" } });
  rerender({ id: "new" });
  await waitFor(() => expect(result.current.models).toEqual(models));
  await act(async () => finish([{ ...models[0], id: "obsolete" }]));
  expect(result.current.models).toEqual(models);
});

it("retries after reconnect and ignores an older request that fails late", async () => {
  let reject!: (error: Error) => void;
  vi.mocked(chatModels).mockImplementationOnce(() => new Promise((_, fail) => { reject = fail; }))
    .mockResolvedValue(models);
  const { result } = renderHook(() => useChatModels("s", 0));
  act(() => vi.mocked(onTransportReconnect).mock.calls[0][0]());
  await waitFor(() => expect(result.current.models).toEqual(models));
  await act(async () => reject(new Error("late failure")));
  expect(result.current.failed).toBe(false);
  expect(result.current.loading).toBe(false);
  expect(chatModels).toHaveBeenCalledTimes(2);
});

it("ends loading after 30 seconds and retries without accepting the timed-out response", async () => {
  vi.useFakeTimers();
  let finish!: (value: ChatModel[]) => void;
  vi.mocked(chatModels).mockImplementationOnce(() => new Promise(resolve => { finish = resolve; }))
    .mockResolvedValue(models);
  const { result } = renderHook(() => useChatModels("s", 0));
  await act(async () => { vi.advanceTimersByTime(29_999); });
  expect(result.current.loading).toBe(true);
  await act(async () => { vi.advanceTimersByTime(1); });
  expect(result.current.loading).toBe(false);
  expect(result.current.failed).toBe(true);
  await act(async () => { result.current.retry(); });
  expect(result.current.models).toEqual(models);
  expect(result.current.failed).toBe(false);
  await act(async () => finish([{ ...models[0], id: "obsolete" }]));
  expect(result.current.models).toEqual(models);
  expect(result.current.failed).toBe(false);
});

it("gives a new session its own deadline and clears the timer on unmount", async () => {
  vi.useFakeTimers();
  vi.mocked(chatModels).mockImplementation(() => new Promise(() => {}));
  const { result, rerender, unmount } = renderHook(({ id }) => useChatModels(id, 0), { initialProps: { id: "old" } });
  await act(async () => { vi.advanceTimersByTime(20_000); });
  rerender({ id: "new" });
  await act(async () => { vi.advanceTimersByTime(10_000); });
  expect(result.current.loading).toBe(true);
  expect(result.current.failed).toBe(false);
  await act(async () => { vi.advanceTimersByTime(20_000); });
  expect(result.current.loading).toBe(false);
  expect(result.current.failed).toBe(true);
  act(() => result.current.retry());
  expect(vi.getTimerCount()).toBe(1);
  unmount();
  expect(vi.getTimerCount()).toBe(0);
});

it("clears a successful request's deadline and preserves its models when a refresh times out", async () => {
  vi.useFakeTimers();
  vi.mocked(chatModels).mockResolvedValueOnce(models)
    .mockImplementationOnce(() => new Promise(() => {}));
  const { result, rerender } = renderHook(({ version }) => useChatModels("s", version), { initialProps: { version: 0 } });
  await act(async () => {});
  expect(result.current.models).toEqual(models);
  expect(vi.getTimerCount()).toBe(0);
  rerender({ version: 1 });
  expect(result.current.models).toEqual(models);
  expect(result.current.loading).toBe(true);
  await act(async () => { vi.advanceTimersByTime(30_000); });
  expect(result.current.models).toEqual(models);
  expect(result.current.loading).toBe(false);
  expect(result.current.failed).toBe(true);
});

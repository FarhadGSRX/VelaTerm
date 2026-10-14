import { beforeEach, expect, it, vi } from "vitest";
vi.mock("../ipc/chat", () => ({ chatSend: vi.fn(), resolveChatImage: vi.fn() }));
vi.mock("../ipc/translation", () => ({ prepareTranslation: vi.fn(), sendTranslation: vi.fn(), cancelTranslation: vi.fn() }));
import { chatSend } from "../ipc/chat";
import { cancelTranslation, prepareTranslation, sendTranslation } from "../ipc/translation";
import { cancelSubmissionTranslation, createSubmission, deliverSubmission, retrySubmission, sendOriginalSubmission, submissionsFor, useOutbox } from "../layout/CenterPane/session/outbox";

beforeEach(() => { vi.clearAllMocks(); useOutbox.setState({ sessions: {} }); vi.mocked(cancelTranslation).mockResolvedValue(); });
it("translates once, preserves attachment and behavior identity, and reuses ready text on rejected delivery", async () => {
  vi.mocked(prepareTranslation).mockResolvedValue({ id: "fixture" });
  vi.mocked(sendTranslation).mockRejectedValueOnce(new Error("Rejected")).mockResolvedValueOnce("queued");
  const images = [{ mimeType: "image/png", data: "fixture" }]; const item = createSubmission("s", "Original", images, "interrupt", true);
  await deliverSubmission("s", item); await retrySubmission("s", submissionsFor("s")[0]);
  expect(prepareTranslation).toHaveBeenCalledTimes(1); expect(sendTranslation).toHaveBeenLastCalledWith("s", item.id, "Original", images, "interrupt"); expect(chatSend).not.toHaveBeenCalled();
});
it("a canceled late result never sends; a fresh retry can proceed before that old request returns", async () => {
  let late!: (value: {id: string}) => void;
  vi.mocked(prepareTranslation).mockImplementationOnce(() => new Promise(resolve => { late = resolve; })).mockResolvedValueOnce({ id: "fixture" });
  vi.mocked(sendTranslation).mockResolvedValue("sent");
  const item = createSubmission("s", "Original", [], "steer", true); const pending = deliverSubmission("s", item);
  await cancelSubmissionTranslation("s", item); await retrySubmission("s", submissionsFor("s")[0]);
  late({ id: "old" }); await pending; expect(sendTranslation).toHaveBeenCalledTimes(1);
});
it("translation failure stays unsent until the user explicitly sends the original", async () => {
  vi.mocked(prepareTranslation).mockRejectedValue(new Error("translation:network")); vi.mocked(chatSend).mockResolvedValue("sent");
  const item = createSubmission("s", "Original", [], "queue", true); await deliverSubmission("s", item);
  expect(submissionsFor("s")[0].status).toBe("translationFailed"); expect(chatSend).not.toHaveBeenCalled();
  await sendOriginalSubmission("s", submissionsFor("s")[0]); expect(chatSend).toHaveBeenCalledWith("s", "Original", "queue", undefined, item.id, true);
});

it("a stale client rejected by the host recovers the original as a translation failure", async () => {
  vi.mocked(chatSend).mockRejectedValue(new Error("translation:required"));
  const item = createSubmission("s", "Original", [], "queue"); await deliverSubmission("s", item);
  expect(submissionsFor("s")[0]).toMatchObject({ text: "Original", translation: true, status: "translationFailed" });
});


it("waits for translation and original mapping before starting or sending to the agent", async () => {
  let prepared!: (value: { id: string }) => void;
  let installed!: () => void;
  const order: string[] = [];
  vi.mocked(prepareTranslation).mockImplementation(() => new Promise(resolve => { prepared = resolve; }));
  vi.mocked(sendTranslation).mockImplementation(async () => { order.push("send"); return "sent"; });
  const start = vi.fn(async () => { order.push("start"); });
  const refresh = vi.fn(() => new Promise<void>(resolve => { installed = () => { order.push("mapping"); resolve(); }; }));
  const item = createSubmission("s", "Original", [], "queue", true);
  const pending = deliverSubmission("s", item, start, refresh);
  expect(start).not.toHaveBeenCalled(); expect(sendTranslation).not.toHaveBeenCalled();
  prepared({ id: item.id }); await Promise.resolve();
  expect(refresh).toHaveBeenCalledTimes(1); expect(start).not.toHaveBeenCalled(); expect(sendTranslation).not.toHaveBeenCalled();
  installed(); await pending;
  expect(order).toEqual(["mapping", "start", "send"]);
});

it("keeps a prepared message unsent if its display mapping cannot load, and retries with the same id", async () => {
  vi.mocked(prepareTranslation).mockResolvedValue({ id: "fixture" });
  vi.mocked(sendTranslation).mockResolvedValue("sent");
  const refresh = vi.fn().mockRejectedValueOnce(new Error("translation:unavailable")).mockResolvedValue(undefined);
  const start = vi.fn().mockResolvedValue(undefined);
  const item = createSubmission("s", "Original", [], "queue", true);
  await deliverSubmission("s", item, start, refresh);
  expect(start).not.toHaveBeenCalled(); expect(sendTranslation).not.toHaveBeenCalled();
  expect(submissionsFor("s")[0]).toMatchObject({ id: item.id, translationReady: true, status: "failed" });
  await retrySubmission("s", submissionsFor("s")[0], start, refresh);
  expect(prepareTranslation).toHaveBeenCalledTimes(1);
  expect(sendTranslation).toHaveBeenCalledExactlyOnceWith("s", item.id, "Original", [], "queue");
});

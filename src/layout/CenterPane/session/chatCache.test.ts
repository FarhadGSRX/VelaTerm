import { expect, it } from "vitest";
import { registerRowPositions, type ChatRow, type ChatSnapshot } from "../../../ipc/chat";
import { mergeRows, reconcileChat } from "./chatCache";

const row = (id: string, position: number): ChatRow => {
  const value: ChatRow = { kind: "user", id, text: id };
  registerRowPositions([value], { [id]: position });
  return value;
};

it("keeps authoritative order and ignores unseen updates before the loaded history window", () => {
  const loaded = [row("middle", 90), row("last", 100)];
  const result = mergeRows(loaded, [row("new", 101), row("old", 2), row("last", 100)], true);
  expect(result.map(item => item.id)).toEqual(["middle", "last", "new"]);
  expect(mergeRows([row("older", 89)], result).map(item => item.id)).toEqual(["older", "middle", "last", "new"]);
});

it("retains recent events over a delayed snapshot without creating gaps in history", () => {
  const baseline = { pageKind: "recent", hasMore: true, startedAt: 100, rowsRevision: 10, queueRevision: 10, rows: [row("middle", 90)], queue: [] } as unknown as ChatSnapshot;
  const result = reconcileChat(baseline, [], [
    { type: "rows", epoch: 100, revision: 11, rows: [row("old", 2), row("new", 91)] },
    { type: "queued", epoch: 100, revision: 12, items: [{ id: "q", text: "queued" }] },
  ]);
  expect(result.rows.map(item => item.id)).toEqual(["middle", "new"]);
  expect(result.queue[0].id).toBe("q");
  expect(result.hasMore).toBe(true);
});

it("merges reconnect deltas and discards events from older processes", () => {
  const baseline = { pageKind: "delta", startedAt: 200, rowsRevision: 20, rows: [row("new", 91)], queue: [] } as unknown as ChatSnapshot;
  const result = reconcileChat(baseline, [row("middle", 90)], [{ type: "replaceRows", epoch: 100, revision: 99, rows: [] }]);
  expect(result.rows.map(item => item.id)).toEqual(["middle", "new"]);
});

it("restores a restarted process atomically while a previous snapshot is in flight", () => {
  const baseline = { startedAt: 100, rowsRevision: 50, rows: [row("old-process", 0)], queue: [] } as unknown as ChatSnapshot;
  const restored = row("restored-history", 90);
  const result = reconcileChat(baseline, [], [
    { type: "reset", epoch: 200, revision: 1, rows: [restored], hasMore: true },
    { type: "rows", epoch: 200, revision: 2, rows: [row("new-message", 91)] },
  ]);
  expect(result.rows.map(item => item.id)).toEqual(["restored-history", "new-message"]);
  expect(result.hasMore).toBe(true);
  expect(result.startedAt).toBe(200);
  expect(result.rowsRevision).toBe(2);
  const newer = { ...result, rowsRevision: 3 };
  expect(reconcileChat(newer, [], [
    { type: "reset", epoch: 200, revision: 1, rows: [restored] },
  ]).rows).toEqual(result.rows);
});


it("keeps a newer authorization event over a late snapshot and clears it on process reset", () => {
  const snapshot = { running: true, rows: [], queue: [], permissions: [], commands: [], configKeys: [], startedAt: 100, auth: { status: "required" as const } };
  const pending = { status: "pending" as const, verificationUrl: "https://auth.openai.com/codex/device", userCode: "ABCD-1234" };
  const updated = reconcileChat(snapshot, [], [{ type: "extras", extras: { auth: pending } }]);
  expect(updated.auth).toEqual(pending);
  expect(reconcileChat(updated, [], [{ type: "reset", epoch: 200, rows: [], revision: 0 }]).auth).toBeUndefined();
});

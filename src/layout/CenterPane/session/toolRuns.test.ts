//! Tests for the folding rules and for where the mounted tail of a long conversation begins.

import { describe, expect, it } from "vitest";

import type { ChatRow } from "../../../ipc/chat";
import {
  estimateRowHeight,
  groupToolRuns,
  markAgentTurns,
  mountedStart,
  runSummaryText,
  type DisplayRow,
  type ToolRow,
} from "./toolRuns";

function tool(id: string, name: string, status: "running" | "completed" = "completed"): ToolRow {
  return { kind: "tool", id, name, input: {}, isError: false, status };
}

function user(id: string, text = "hi"): ChatRow {
  return { kind: "user", id, text };
}

function assistant(id: string, text = "there"): ChatRow {
  return { kind: "assistant", id, text, streaming: false };
}

/** The shape of the result, as `kind:id`, which is what the assertions are really about. */
function shape(rows: DisplayRow[]): string[] {
  return rows.map((row) => `${row.kind}:${row.id}`);
}

describe("groupToolRuns", () => {
  it("folds three or more consecutive calls into one run", () => {
    const rows = [user("u1"), tool("t1", "Read"), tool("t2", "Read"), tool("t3", "Grep"), assistant("a1")];
    const out = groupToolRuns(rows);
    expect(shape(out)).toEqual(["row:u1", "run:t1", "row:a1"]);
    expect(out[1].kind === "run" && out[1].calls.length).toBe(3);
  });

  it("leaves a short burst as separate cards", () => {
    const rows = [tool("t1", "Read"), tool("t2", "Read"), assistant("a1")];
    expect(shape(groupToolRuns(rows))).toEqual(["row:t1", "row:t2", "row:a1"]);
  });

  it("keeps the calls that are still running out of the fold", () => {
    const rows = [
      tool("t1", "Read"),
      tool("t2", "Read"),
      tool("t3", "Read"),
      tool("t4", "Bash", "running"),
    ];
    const out = groupToolRuns(rows);
    expect(shape(out)).toEqual(["run:t1", "row:t4"]);
  });

  it("folds nothing when taking the running calls out leaves too few", () => {
    const rows = [tool("t1", "Read"), tool("t2", "Read"), tool("t3", "Bash", "running")];
    expect(shape(groupToolRuns(rows))).toEqual(["row:t1", "row:t2", "row:t3"]);
  });

  it("holds a run that is still running open, so its state shows on the folded row", () => {
    const rows = [
      tool("t1", "Read", "running"),
      tool("t2", "Read"),
      tool("t3", "Read"),
      tool("t4", "Read"),
      assistant("a1"),
    ];
    const out = groupToolRuns(rows);
    expect(out[0].kind === "run" && out[0].running).toBe(true);
  });

  it("starts a new run after anything that is not a tool call", () => {
    const rows = [
      tool("t1", "Read"),
      tool("t2", "Read"),
      tool("t3", "Read"),
      assistant("a1"),
      tool("t4", "Read"),
      tool("t5", "Read"),
      tool("t6", "Read"),
    ];
    expect(shape(groupToolRuns(rows))).toEqual(["run:t1", "row:a1", "run:t4"]);
  });

  it("leaves a conversation with no tool calls untouched", () => {
    const rows = [user("u1"), assistant("a1")];
    expect(shape(groupToolRuns(rows))).toEqual(["row:u1", "row:a1"]);
  });
});

describe("markAgentTurns", () => {
  it("marks the first entry of a turn and fills the line from its replies", () => {
    const display = groupToolRuns([
      user("u1"),
      { kind: "reasoning", id: "r1", text: "hmm", streaming: false },
      tool("t1", "Read"),
      tool("t2", "Read"),
      tool("t3", "Read"),
      { kind: "assistant", id: "a1", text: "done", streaming: false, model: "m", at: 12_345, durationMs: 3_000 },
      user("u2"),
      assistant("a2"),
    ]);
    const out = markAgentTurns(display);
    expect(out[0].head).toBeUndefined();
    expect(out[1].head).toEqual({ who: "m", at: 12_345, durationMs: 3_000 });
    expect(out[2].head).toBeUndefined();
    expect(out[2].kind === "run" && out[2].calls.length).toBe(3);
    expect(out[3].head).toBeUndefined();
    expect(out[4].head).toBeUndefined();
    expect(out[5].head).toEqual({});
  });

  it("leaves remarks about the conversation without an author line", () => {
    const display = groupToolRuns([
      user("u1"),
      { kind: "notice", id: "n1", message: "retrying" },
      { kind: "error", id: "e1", message: "failed" },
      assistant("a1"),
    ]);
    const out = markAgentTurns(display);
    expect(out[1].head).toBeUndefined();
    expect(out[2].head).toBeUndefined();
    expect(out[3].head).toEqual({});
  });

  it("marks a turn that was cut off at the top of history", () => {
    const display = groupToolRuns([
      { kind: "reasoning", id: "r1", text: "hmm", streaming: false },
      assistant("a1"),
    ]);
    expect(markAgentTurns(display)[0].head).toEqual({});
  });

  it("counts the turn's line into the height estimate", () => {
    const plain: DisplayRow = { kind: "row", id: "a", row: assistant("a") };
    expect(estimateRowHeight({ ...plain, head: {} })).toBe(estimateRowHeight(plain) + 34);
  });
});

describe("runSummaryText", () => {
  it("counts each tool in the order it first appears", () => {
    const calls = [tool("1", "Read"), tool("2", "Read"), tool("3", "Grep"), tool("4", "Read")];
    expect(runSummaryText(calls)).toBe("Read ×3 · Grep");
  });
});

describe("mountedStart", () => {
  const many = (n: number) => groupToolRuns(Array.from({ length: n }, (_, i) => assistant(`a${i}`)));

  it("mounts everything while the conversation is short", () => {
    expect(mountedStart(many(10), 30)).toBe(0);
  });

  it("walks back to the start of a turn so the tail begins at a user message", () => {
    const rows: ChatRow[] = [];
    for (let i = 0; i < 40; i += 1) rows.push(assistant(`a${i}`));
    rows[33] = user("u33");
    const display = groupToolRuns(rows);
    expect(mountedStart(display, 5)).toBe(33);
  });

  it("stops walking back rather than mounting a turn of any length", () => {
    const display = many(100);
    expect(mountedStart(display, 10)).toBe(80);
  });
});

describe("estimateRowHeight", () => {
  it("guesses taller for longer prose", () => {
    const short = estimateRowHeight({ kind: "row", id: "a", row: assistant("a", "hi") });
    const long = estimateRowHeight({ kind: "row", id: "b", row: assistant("b", "x".repeat(800)) });
    expect(long).toBeGreaterThan(short);
  });

  it("treats a folded run as one short row", () => {
    const run = groupToolRuns([tool("t1", "Read"), tool("t2", "Read"), tool("t3", "Read")])[0];
    expect(estimateRowHeight(run)).toBeLessThan(60);
  });
});

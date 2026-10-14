import { expect, it } from "vitest";
import { layoutGraph, parseRefs } from "./gitGraph";

it("keeps linear history in one lane", () => {
  const rows = layoutGraph([{ oid: "c", parents: ["b"] }, { oid: "b", parents: ["a"] }, { oid: "a", parents: [] }]);
  expect(rows.map((r) => [r.col, r.width, r.incoming, r.outgoing])).toEqual([
    [0, 1, [], [0]],
    [0, 1, [0], [0]],
    [0, 1, [0], []],
  ]);
});

it("forks for a merge and joins the side branch back at its base", () => {
  // m merges f into the main line; f and b both descend from a.
  const rows = layoutGraph([
    { oid: "m", parents: ["b", "f"] },
    { oid: "f", parents: ["a"] },
    { oid: "b", parents: ["a"] },
    { oid: "a", parents: [] },
  ]);
  expect(rows[0]).toMatchObject({ col: 0, outgoing: [0, 1], width: 2 });
  expect(rows[1]).toMatchObject({ col: 1, through: [0], incoming: [1], outgoing: [1] });
  // b continues lane 0 while lane 1 still waits for a.
  expect(rows[2]).toMatchObject({ col: 0, through: [1], incoming: [0], outgoing: [0] });
  // Both lanes end at a.
  expect(rows[3]).toMatchObject({ col: 0, incoming: [0, 1], outgoing: [], through: [] });
});

it("reuses a freed column for the next new lane", () => {
  const rows = layoutGraph([
    { oid: "x", parents: ["a"] },
    { oid: "y", parents: ["a"] },
    { oid: "a", parents: [] },
    { oid: "z", parents: [] },
  ]);
  expect(rows[1].col).toBe(1);
  expect(rows[3].col).toBe(0);
});

it("reads decorations", () => {
  expect(parseRefs(["HEAD -> main", "origin/main", "origin/HEAD", "tag: v1.0", "feature/x"])).toEqual([
    { name: "main", kind: "head" },
    { name: "origin/main", kind: "branch" },
    { name: "v1.0", kind: "tag" },
    { name: "feature/x", kind: "branch" },
  ]);
});

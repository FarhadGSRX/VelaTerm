// Layout cases adapted from Branch Visualizer's `src/lib/graph.check.ts`
// (https://github.com/wannabeepolymath/git-branch-Visualizer, MIT License, Copyright (c) 2026 Daksh).
import { describe, expect, it } from "vitest";
import { lanesBelow, layoutGraph, refLabels, type GraphCommit } from "./commitGraph";

const c = (id: string, ...parents: string[]): GraphCommit => ({ id, parents });

describe("layoutGraph", () => {
  it("keeps a linear history in one lane", () => {
    const { rows, maxLanes } = layoutGraph([c("c3", "c2"), c("c2", "c1"), c("c1")]);
    expect(rows.every((r) => r.lane === 0)).toBe(true);
    expect(rows.every((r) => r.passes.length === 0)).toBe(true);
    expect(rows[0].ins).toEqual([]);
    expect(rows[2].outs).toEqual([]);
    expect(maxLanes).toBe(1);
  });

  it("connects a merge commit to both parent lanes", () => {
    const { rows, maxLanes } = layoutGraph([
      c("m", "a", "b"),
      c("a", "base"),
      c("b", "base"),
      c("base"),
    ]);
    expect(rows[0].lane).toBe(0);
    expect(rows[0].outs).toEqual([0, 1]);
    expect(rows[1].passes).toEqual([1]);
    expect(rows[2].lane).toBe(1);
    expect(rows[3].ins).toEqual([0, 1]);
    expect(rows[3].lane).toBe(0);
    expect(maxLanes).toBe(2);
  });

  it("reuses a freed lane for a later tip", () => {
    // t is an octopus merge across lanes 0-2; x is a root, which frees lane 1 for the tip w.
    const { rows, maxLanes } = layoutGraph([
      c("t", "a", "x", "y"),
      c("x"),
      c("w", "y"),
      c("y"),
      c("a"),
    ]);
    expect(rows[0].outs).toEqual([0, 1, 2]);
    expect(rows[1].outs).toEqual([]);
    expect(rows[2].lane).toBe(1);
    expect(rows[3].ins).toEqual([1, 2]);
    expect(rows[3].lane).toBe(1);
    expect(maxLanes).toBe(3);
  });

  it("leaves an edge running off the bottom when the parent is on a later page", () => {
    const { rows, maxLanes } = layoutGraph([c("m", "a", "not-loaded"), c("a")]);
    expect(rows[0].outs).toEqual([0, 1]);
    expect(rows[1].passes).toEqual([1]);
    expect(rows[1].outs).toEqual([]);
    expect(maxLanes).toBe(2);
  });

  it("continues every open lane below an expanded row", () => {
    const { rows } = layoutGraph([c("m", "a", "b"), c("a", "base"), c("b", "base"), c("base")]);
    expect(lanesBelow(rows[0])).toEqual([0, 1]);
    expect(lanesBelow(rows[1])).toEqual([0, 1]);
    expect(lanesBelow(rows[3])).toEqual([]);
  });
});

describe("refLabels", () => {
  it("shortens full ref names and tells their kinds apart", () => {
    expect(
      refLabels([
        "refs/remotes/origin/main",
        "HEAD -> refs/heads/main",
        "refs/heads/feature/x",
        "tag: refs/tags/v1",
      ]),
    ).toEqual([
      { name: "main", kind: "head" },
      { name: "feature/x", kind: "branch" },
      { name: "v1", kind: "tag" },
      { name: "origin/main", kind: "remote" },
    ]);
  });

  it("labels a detached HEAD and drops the stash and remote HEAD pointers", () => {
    expect(refLabels(["HEAD", "refs/stash", "refs/remotes/origin/HEAD"])).toEqual([
      { name: "HEAD", kind: "head" },
    ]);
  });
});

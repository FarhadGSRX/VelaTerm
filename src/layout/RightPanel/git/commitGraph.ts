//! Lane layout for the commit graph in the Git panel's history.
//!
//! Ported from Branch Visualizer by Daksh (https://github.com/wannabeepolymath/git-branch-Visualizer),
//! `src/lib/graph.ts`, used under the MIT License:
//!
//!   Copyright (c) 2026 Daksh
//!
//!   Permission is hereby granted, free of charge, to any person obtaining a copy of this software
//!   and associated documentation files (the "Software"), to deal in the Software without
//!   restriction, including without limitation the rights to use, copy, modify, merge, publish,
//!   distribute, sublicense, and/or sell copies of the Software, and to permit persons to whom the
//!   Software is furnished to do so, subject to the following conditions:
//!
//!   The above copyright notice and this permission notice shall be included in all copies or
//!   substantial portions of the Software.
//!
//!   THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING
//!   BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND
//!   NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM,
//!   DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
//!   OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.
//!
//! Input is commits in `git log --topo-order` order, newest first, so every child comes before its
//! parents. Each output row says which lane holds the commit and which lanes enter, leave, or pass
//! through it, so a row can be drawn on its own without looking at its neighbours.

export interface GraphCommit {
  /** Full hash. */
  id: string;
  /** Full parent hashes, first parent first. */
  parents: string[];
}

export interface GraphRow {
  /** Lane of this commit's node. */
  lane: number;
  /** Lanes entering from the top edge that end at this node (children above). */
  ins: number[];
  /** Lanes leaving the node toward the bottom edge (parents below, or past the loaded page). */
  outs: number[];
  /** Lanes passing straight through this row without touching the node. */
  passes: number[];
}

export interface GraphLayout {
  rows: GraphRow[];
  maxLanes: number;
}

/**
 * Lane colours. They are theme tokens rather than fixed hex values, so the graph follows the
 * dark and light theme pickers like every other accent in the panel.
 */
const LANE_COLORS = [
  "var(--accent)",
  "var(--green)",
  "var(--mag)",
  "var(--yellow)",
  "var(--cyan)",
  "var(--red)",
  "var(--blue)",
];

export function laneColor(lane: number): string {
  return LANE_COLORS[lane % LANE_COLORS.length];
}

export function layoutGraph(commits: readonly GraphCommit[]): GraphLayout {
  // lanes[j] = the commit this lane is waiting for (its next node), or null if the lane is free.
  const lanes: (string | null)[] = [];
  const rows: GraphRow[] = [];
  let maxLanes = 0;

  const firstFree = (): number => {
    const i = lanes.indexOf(null);
    if (i !== -1) return i;
    lanes.push(null);
    return lanes.length - 1;
  };

  for (const c of commits) {
    const activeBefore = lanes.map((h) => h !== null);

    // Every lane waiting for this commit is a child edge coming in from above.
    const ins: number[] = [];
    for (let j = 0; j < lanes.length; j++) if (lanes[j] === c.id) ins.push(j);

    // The node sits on the leftmost waiting lane; a commit nobody waits for is a tip and opens one.
    const lane = ins.length > 0 ? ins[0] : firstFree();
    for (let k = 1; k < ins.length; k++) lanes[ins[k]] = null; // the other children merge in here

    // The first parent continues in the node's lane. Each further (merge) parent gets a lane of
    // its own, reusing one that already waits for the same parent.
    const outs: number[] = [];
    const firstParent = c.parents.length > 0 ? c.parents[0] : null;
    lanes[lane] = firstParent; // a root commit frees its lane
    if (firstParent !== null) outs.push(lane);
    for (let pi = 1; pi < c.parents.length; pi++) {
      const p = c.parents[pi];
      let pj = lanes.indexOf(p);
      if (pj === -1) {
        pj = firstFree();
        lanes[pj] = p;
      }
      if (!outs.includes(pj)) outs.push(pj);
    }

    // Lanes active before and after this row that the node does not touch pass through. A parent
    // lane that already existed both passes through and receives an out curve, which is what a
    // merge into an existing lane looks like.
    const passes: number[] = [];
    for (let j = 0; j < lanes.length; j++) {
      if (j !== lane && activeBefore[j] && lanes[j] !== null && !ins.includes(j)) passes.push(j);
    }

    let width = lane + 1;
    for (const j of ins) width = Math.max(width, j + 1);
    for (const j of outs) width = Math.max(width, j + 1);
    for (const j of passes) width = Math.max(width, j + 1);
    maxLanes = Math.max(maxLanes, width);

    rows.push({ lane, ins, outs, passes });

    while (lanes.length > 0 && lanes[lanes.length - 1] === null) lanes.pop();
  }

  return { rows, maxLanes };
}

/** Lanes still open below a row: the ones its expanded file list has to keep drawing. */
export function lanesBelow(row: GraphRow): number[] {
  return [...new Set([...row.outs, ...row.passes])].sort((a, b) => a - b);
}

/** A ref label on a history row: the checked-out branch, another local branch, a remote branch, or a tag. */
export interface RefLabel {
  name: string;
  kind: "head" | "branch" | "remote" | "tag";
}

/**
 * Turn git's full decorations into short labels. The stash and remote HEAD pointers are dropped:
 * they name no branch and only crowd a narrow row.
 */
export function refLabels(refs: readonly string[]): RefLabel[] {
  const out: RefLabel[] = [];
  for (const raw of refs) {
    let r = raw;
    let head = false;
    if (r === "HEAD") {
      // Detached HEAD: no branch to name, so the label is HEAD itself.
      out.push({ name: "HEAD", kind: "head" });
      continue;
    }
    if (r.startsWith("HEAD -> ")) {
      head = true;
      r = r.slice("HEAD -> ".length);
    }
    if (r.startsWith("tag: ")) {
      out.push({ name: r.slice("tag: ".length).replace(/^refs\/tags\//, ""), kind: "tag" });
    } else if (r.startsWith("refs/heads/")) {
      out.push({ name: r.slice("refs/heads/".length), kind: head ? "head" : "branch" });
    } else if (r.startsWith("refs/remotes/")) {
      if (r.endsWith("/HEAD")) continue;
      out.push({ name: r.slice("refs/remotes/".length), kind: "remote" });
    } else if (r !== "refs/stash") {
      out.push({ name: r, kind: head ? "head" : "branch" });
    }
  }
  // The checked-out branch reads first, then local branches, tags, and remote copies last: a narrow
  // row has room for one label, and the local name says more than its remote twin.
  const rank = { head: 0, branch: 1, tag: 2, remote: 3 };
  return out.sort((a, b) => rank[a.kind] - rank[b.kind]);
}

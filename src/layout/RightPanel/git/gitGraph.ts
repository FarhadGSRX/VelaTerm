//! Lane layout for the commit history graph. Each commit gets a column; lines carry each branch of
//! history down to the commit that continues it. Columns never move once assigned — a lane that
//! ends leaves a gap the next new lane reuses — so a line is straight unless it really forks or
//! joins, and the result can be extended page by page without reshuffling what is already drawn.

/** The parts of a commit the layout needs. */
export interface GraphCommit {
  oid: string;
  parents: string[];
}

export interface GraphRow {
  /** Column of this commit's node. */
  col: number;
  /** Lanes passing straight through this row, top to bottom. */
  through: number[];
  /** Lanes that end at this commit, drawn from the top edge to the node; includes `col` unless the
   *  commit starts a new lane (a branch tip). */
  incoming: number[];
  /** Lanes leaving the node toward this commit's parents. */
  outgoing: number[];
  /** Columns in use in this row, for the drawing width. */
  width: number;
  /** Colour index of the node's lane. */
  color: number;
}

/** Number of distinct lane colours; lanes cycle through them by column. */
export const GRAPH_COLORS = 6;

export function layoutGraph(commits: GraphCommit[]): GraphRow[] {
  // Each slot holds the commit the lane is waiting for, or null when free.
  const lanes: (string | null)[] = [];
  const rows: GraphRow[] = [];
  const free = () => {
    const i = lanes.indexOf(null);
    return i >= 0 ? i : lanes.push(null) - 1;
  };
  for (const commit of commits) {
    const waiting = lanes.flatMap((oid, i) => (oid === commit.oid ? [i] : []));
    const col = waiting.length ? waiting[0] : free();
    const through = lanes.flatMap((oid, i) => (oid !== null && oid !== commit.oid ? [i] : []));
    for (const i of waiting) lanes[i] = null;
    const outgoing: number[] = [];
    commit.parents.forEach((parent, n) => {
      // The first parent always continues this lane, so a main line stays straight even when a side
      // lane waits for the same commit; the two meet there. Other parents join a lane already
      // waiting for them, or open one.
      const existing = lanes.indexOf(parent);
      if (n === 0) {
        lanes[col] = parent;
        outgoing.push(col);
      } else if (existing >= 0) {
        outgoing.push(existing);
      } else {
        const slot = free();
        lanes[slot] = parent;
        outgoing.push(slot);
      }
    });
    while (lanes.length && lanes[lanes.length - 1] === null) lanes.pop();
    const width = Math.max(col + 1, ...through.map((i) => i + 1), ...outgoing.map((i) => i + 1), ...waiting.map((i) => i + 1));
    rows.push({ col, through, incoming: waiting, outgoing, width, color: col % GRAPH_COLORS });
  }
  return rows;
}

/** Split `%D` decorations into what a row shows: the checked-out branch, other branches, and tags.
 *  `%D` does not say whether `a/b` is a remote branch or a local one with a slash, so both are
 *  shown as branches. */
export function parseRefs(refs: string[] | undefined): { name: string; kind: "head" | "branch" | "tag" }[] {
  const out: { name: string; kind: "head" | "branch" | "tag" }[] = [];
  for (const raw of refs ?? []) {
    if (raw === "HEAD" || raw.endsWith("/HEAD")) continue;
    if (raw.startsWith("HEAD -> ")) out.push({ name: raw.slice(8), kind: "head" });
    else if (raw.startsWith("tag: ")) out.push({ name: raw.slice(5), kind: "tag" });
    else out.push({ name: raw, kind: "branch" });
  }
  return out;
}

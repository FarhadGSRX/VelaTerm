//! Commit history for the Git panel: a page of commits at a time, each expandable into the files it
//! touched. Clicking a file opens the Changes modal in commit mode, so committed and uncommitted
//! diffs are read in the same viewer.
//!
//! A graph rail runs down the left of the list, drawn one row at a time from the lane layout in
//! `commitGraph.ts`, with the branch and tag labels each commit carries.
//!
//! History is loaded on demand rather than with the rest of the tab: a repository can hold hundreds
//! of thousands of commits, and the tab polls every few seconds.

import { useCallback, useEffect, useMemo, useState } from "react";
import Icons from "../../../components/Icons";
import { statusLetter } from "../../../components/diff/changeStatus";
import { useT } from "../../../i18n";
import {
  gitCommitFiles,
  gitLogPage,
  type ChangedFile,
  type CommitInfo,
} from "../../../ipc/commands";
import { useTermStore } from "../../../store/termStore";
import {
  laneColor,
  lanesBelow,
  layoutGraph,
  refLabels,
  type GraphRow,
} from "./commitGraph";

/** Commits fetched per page. Enough to fill the panel without holding a whole history in memory. */
const PAGE = 30;

/** Height of one commit row; the rail is drawn to the same height so its lines meet across rows. */
const ROW_H = 22;
/** Preferred gap between lanes, and the width the rail may grow to before lanes squeeze together. */
const LANE_W = 8;
const RAIL_MAX = 48;
/** Space left of the first lane and right of the last. */
const RAIL_PAD = 5;
/** Ref labels shown inline before the rest collapse into a "+n" count; the panel is narrow. */
const MAX_REFS = 1;

/**
 * Lane geometry for the loaded commits. Wide graphs squeeze lanes closer, down to a floor; past
 * that the rail stops growing and the outermost lanes are clipped, so a busy repository cannot push
 * the commit subjects out of a narrow panel.
 */
function railGeometry(maxLanes: number) {
  const lanes = Math.max(1, maxLanes);
  const gap = lanes > 1 ? Math.max(4, Math.min(LANE_W, RAIL_MAX / (lanes - 1))) : LANE_W;
  const width = Math.min(RAIL_MAX, (lanes - 1) * gap) + RAIL_PAD * 2;
  return { width, x: (lane: number) => RAIL_PAD + lane * gap };
}

type Geometry = ReturnType<typeof railGeometry>;

/** One row's slice of the graph: lines passing through, edges in and out, and the commit's node. */
function Rail({ row, geo, merge }: { row: GraphRow; geo: Geometry; merge: boolean }) {
  const cx = geo.x(row.lane);
  const cy = ROW_H / 2;
  const paths: React.ReactNode[] = [];
  for (const j of row.passes) {
    paths.push(<path key={`p${j}`} d={`M ${geo.x(j)} 0 V ${ROW_H}`} stroke={laneColor(j)} />);
  }
  for (const j of row.ins) {
    const d =
      j === row.lane ? `M ${cx} 0 V ${cy}` : `M ${geo.x(j)} 0 Q ${geo.x(j)} ${cy} ${cx} ${cy}`;
    paths.push(<path key={`i${j}`} d={d} stroke={laneColor(j)} />);
  }
  for (const j of row.outs) {
    const d =
      j === row.lane
        ? `M ${cx} ${cy} V ${ROW_H}`
        : `M ${cx} ${cy} Q ${geo.x(j)} ${cy} ${geo.x(j)} ${ROW_H}`;
    paths.push(<path key={`o${j}`} d={d} stroke={laneColor(j)} />);
  }
  const c = laneColor(row.lane);
  return (
    <svg className="git-rail" width={geo.width} height={ROW_H} aria-hidden="true">
      <g fill="none" strokeWidth={1.5}>{paths}</g>
      {merge ? (
        <circle cx={cx} cy={cy} r={3} strokeWidth={1.5} stroke={c} fill="var(--bg-1)" />
      ) : (
        <circle cx={cx} cy={cy} r={3} fill={c} />
      )}
    </svg>
  );
}

/** The rail behind an expanded commit's file list: the lanes still open below it, run straight down. */
function RailContinue({ lanes, geo }: { lanes: number[]; geo: Geometry }) {
  return (
    <div className="git-rail-cont" style={{ width: geo.width }} aria-hidden="true">
      {lanes.map((j) => (
        <span key={j} style={{ left: geo.x(j) - 0.75, background: laneColor(j) }} />
      ))}
    </div>
  );
}

/** Branch and tag labels for one commit, the first few inline and the rest counted. */
function RefPills({ refs }: { refs: string[] }) {
  const labels = refLabels(refs);
  if (labels.length === 0) return null;
  const extra = labels.length - MAX_REFS;
  return (
    <>
      {labels.slice(0, MAX_REFS).map((r) => (
        <span key={r.kind + r.name} className={"git-ref git-ref-" + r.kind} title={r.name}>
          {r.name}
        </span>
      ))}
      {extra > 0 && (
        <span
          className="git-ref git-ref-more"
          title={labels
            .slice(MAX_REFS)
            .map((r) => r.name)
            .join(", ")}
        >
          +{extra}
        </span>
      )}
    </>
  );
}

/** Files of one commit, loaded the first time that commit is expanded. */
function CommitFiles({ cwd, hash }: { cwd: string; hash: string }) {
  const t = useT();
  const [files, setFiles] = useState<ChangedFile[] | null>(null);
  const [err, setErr] = useState("");
  const openChanges = useTermStore((s) => s.openChanges);

  useEffect(() => {
    let alive = true;
    void gitCommitFiles(cwd, hash)
      .then((f) => {
        if (alive) setFiles(f);
      })
      .catch((e) => {
        if (alive) setErr(String(e));
      });
    return () => {
      alive = false;
    };
  }, [cwd, hash]);

  if (err) return <div className="insp-empty git-err">{err}</div>;
  if (!files) return <div className="insp-empty">{t("changes.loading")}</div>;
  // A merge commit has no diff against a single parent, so there is nothing to list.
  if (files.length === 0) return <div className="insp-empty">{t("git.commitNoFiles")}</div>;

  return (
    <div className="git-commit-files">
      {files.map((f) => {
        const letter = statusLetter(f.status);
        return (
          <button
            type="button"
            className="git-row-main"
            key={f.path}
            title={f.path}
            onClick={() => openChanges(cwd, { path: f.path, commit: hash })}
          >
            <span className={"gb gb-" + letter}>{letter}</span>
            <span className="nm">
              <span className="base">{f.path}</span>
            </span>
            {!f.binary && (
              <span className="n">
                <span className="plus">+{f.additions}</span>{" "}
                <span className="minus">−{f.deletions}</span>
              </span>
            )}
          </button>
        );
      })}
    </div>
  );
}

export function CommitHistory({
  cwd,
  total,
  tick,
  allBranches,
}: {
  cwd: string;
  total: number;
  /** Panel refresh counter. A new commit shifts every offset, so paged rows must be reloaded, not appended. */
  tick: number;
  /** Walk every branch, remote, and tag instead of the current branch's history alone. */
  allBranches: boolean;
}) {
  const t = useT();
  const [commits, setCommits] = useState<CommitInfo[]>([]);
  const [loading, setLoading] = useState(false);
  const [err, setErr] = useState("");
  const [expanded, setExpanded] = useState<string | null>(null);

  const loadPage = useCallback(
    (offset: number) => {
      setLoading(true);
      gitLogPage(cwd, PAGE, offset, allBranches)
        .then((page) => {
          // Replacing at offset 0 and appending otherwise keeps a refresh from duplicating rows.
          setCommits((cur) => (offset === 0 ? page : [...cur, ...page]));
          setErr("");
        })
        .catch((e) => setErr(String(e)))
        .finally(() => setLoading(false));
    },
    [cwd, allBranches],
  );

  // The layout runs over every loaded page at once: a lane opened on one page can close on the next.
  const graph = useMemo(() => layoutGraph(commits), [commits]);
  const geo = railGeometry(graph.maxLanes);

  // Load the first page when the section mounts, and start over when the repository changes or the
  // panel refreshes. Committing shifts the whole history by one, so keeping the old rows would both
  // hide the new commit and make "Load more" re-append a row that is already on screen.
  useEffect(() => {
    setCommits([]);
    setExpanded(null);
    loadPage(0);
  }, [cwd, loadPage, tick]);

  if (err) return <div className="insp-empty git-err">{err}</div>;
  if (commits.length === 0) {
    return <div className="insp-empty">{loading ? t("changes.loading") : t("git.noCommits")}</div>;
  }

  return (
    <div>
      {commits.map((c, i) => {
        const open = expanded === c.hash;
        const row = graph.rows[i];
        return (
          <div key={c.hash} className="git-commit-item">
            <div className="git-commit-line">
              <Rail row={row} geo={geo} merge={c.parents.length > 1} />
              <button
                type="button"
                className="git-commit-row"
                onClick={() => setExpanded(open ? null : c.hash)}
                aria-expanded={open}
                title={`${c.hash} · ${c.author}`}
              >
                <span className="tw">
                  {open ? <Icons.chevD size={11} /> : <Icons.chevR size={11} />}
                </span>
                <span className="hash">{c.hash}</span>
                <RefPills refs={c.refs} />
                <span className="subj">{c.subject}</span>
                <span className="when">{c.relative}</span>
              </button>
            </div>
            {open && (
              <div className="git-commit-line">
                <RailContinue lanes={lanesBelow(row)} geo={geo} />
                <CommitFiles cwd={cwd} hash={c.hash} />
              </div>
            )}
          </div>
        );
      })}
      {commits.length < total && (
        <button
          type="button"
          className="git-more"
          disabled={loading}
          onClick={() => loadPage(commits.length)}
        >
          {loading ? t("changes.loading") : t("git.loadMore")}
        </button>
      )}
    </div>
  );
}

//! Changes modal showing the selected staged, unstaged, or combined workspace comparison.
//! The left side lists changed files; selecting one renders a line-by-line diff in a CodeMirror MergeView on
//! the right. Mounted at the App root and driven by changesCwd in the store, this self-contained overlay does
//! not interact with center tabs or keep-alive behavior.
//!
//! With changesCommit set the same layout shows one commit instead of the worktree: the file list comes from
//! that commit and each diff compares its parent against it. The Git panel's history opens the modal this way,
//! so committed and uncommitted changes are read through one viewer.

import { useEffect, useRef, useState } from "react";
import type { MergeView } from "@codemirror/merge";
import { useT } from "../../i18n";
import { Backdrop } from "../Backdrop";
import { diffIgnoringWhitespace } from "./whitespaceDiff";
import { replaceChangesPath, useChangesRoute } from "./changesRoute";
import { filesForSide } from "./changeGroups";
import { GitFileRow } from "../../layout/RightPanel/git/GitFileRow";
import "./changes-modal.css";
import {
  gitChangedFiles,
  gitCommitFileDiff,
  gitCommitFiles,
  gitRangeFileDiff,
  gitRangeFiles,
  gitFileDiff,
  type ChangedFile,
  type FileDiff,
  type GitChangeSide,
} from "../../ipc/commands";
import { useTermStore } from "../../store/termStore";


/** Viewer preferences, kept across openings. */
interface DiffOptions {
  /** Treat lines that differ only in whitespace as unchanged. */
  ignoreWhitespace: boolean;
  /** Fold long runs of unchanged lines. */
  collapseUnchanged: boolean;
}

const OPTIONS_KEY = "vlx.diff.options";

function readDiffOptions(): DiffOptions {
  try {
    const saved = JSON.parse(localStorage.getItem(OPTIONS_KEY) ?? "{}") as Partial<DiffOptions>;
    return { ignoreWhitespace: saved.ignoreWhitespace === true, collapseUnchanged: saved.collapseUnchanged !== false };
  } catch {
    return { ignoreWhitespace: false, collapseUnchanged: true };
  }
}

function writeDiffOptions(options: DiffOptions): void {
  try {
    localStorage.setItem(OPTIONS_KEY, JSON.stringify(options));
  } catch {
    // The choice then lasts while the dialog is open.
  }
}

/**
 * Render a single-file diff by loading both texts into a read-only MergeView, adding language support
 * transparently once loaded. `tick` is the modal's refresh counter: it carries no data, it only forces
 * the open diff to reload so it cannot disagree with the refreshed line counts beside it.
 */
function DiffView({
  cwd,
  path,
  tick,
  commit,
  base,
  side,
  options,
}: {
  cwd: string;
  path: string;
  tick: number;
  options: DiffOptions;
  commit: string | null;
  /** Left side of a range comparison; the commit's parent when null. */
  base: string | null;
  side: GitChangeSide;
}) {
  const t = useT();
  const unchangedLines = t("changes.unchangedLines");
  const ref = useRef<HTMLDivElement>(null);
  const [diff, setDiff] = useState<FileDiff | null>(null);
  const [err, setErr] = useState("");

  useEffect(() => {
    let alive = true;
    setDiff(null);
    setErr("");
    const load = commit && base ? gitRangeFileDiff(cwd, base, commit, path)
      : commit ? gitCommitFileDiff(cwd, commit, path) : gitFileDiff(cwd, path, side);
    void load
      .then((d) => {
        if (alive) setDiff(d);
      })
      .catch((e) => {
        if (alive) setErr(String(e));
      });
    return () => {
      alive = false;
    };
  }, [cwd, path, tick, commit, base, side]);

  useEffect(() => {
    const el = ref.current;
    if (!el || !diff || diff.binary || diff.tooLarge) return;
    let mv: MergeView | null = null;
    let cancelled = false;
    void (async () => {
      // CodeMirror and the language catalogue load on demand: the modal is opened rarely, and a static
      // import would place roughly 330 kB of editor code in the entry chunk.
      const [merge, view, state, theme] = await Promise.all([
        import("@codemirror/merge"),
        import("@codemirror/view"),
        import("@codemirror/state"),
        import("./codeMirrorTheme"),
      ]);
      const { MergeView, diff: charDiff, Change } = merge;
      const { EditorView, lineNumbers } = view;
      const { EditorState } = state;
      const langExt = await theme.languageExtensionFor(diff.path);
      if (cancelled || !ref.current) return;
      const base = [
        lineNumbers(),
        EditorView.editable.of(false),
        EditorState.readOnly.of(true),
        EditorState.phrases.of({ "$ unchanged lines": unchangedLines }),
        EditorView.lineWrapping,
        theme.vlxCmHighlighting(),
      ];
      const side = langExt ? [...base, langExt] : base;
      mv = new MergeView({
        a: { doc: diff.original, extensions: side },
        b: { doc: diff.modified, extensions: side },
        parent: ref.current,
        collapseUnchanged: options.collapseUnchanged ? { margin: 3, minSize: 4 } : undefined,
        diffConfig: options.ignoreWhitespace
          ? { override: (a, b) => diffIgnoringWhitespace(a, b, charDiff, Change) ?? charDiff(a, b) }
          : undefined,
        gutter: true,
        highlightChanges: !options.ignoreWhitespace,
      });
    })();
    return () => {
      cancelled = true;
      mv?.destroy();
    };
  }, [diff, unchangedLines, options.collapseUnchanged, options.ignoreWhitespace]);

  if (err) {
    return (
      <div style={{ padding: 16, fontSize: 12.5, color: "var(--danger, #e05252)", whiteSpace: "pre-wrap" }}>
        {err}
      </div>
    );
  }
  if (!diff) {
    return (
      <div style={{ padding: 16, fontSize: 12.5, color: "var(--text-muted)" }}>
        {t("changes.loadingDiff")}
      </div>
    );
  }
  if (diff.tooLarge) return <div className="changes-message">{t("changes.tooLarge")}</div>;
  if (diff.binary) {
    return (
      <div style={{ padding: 16, fontSize: 12.5, color: "var(--text-muted)" }}>
        {t("changes.binary")}
      </div>
    );
  }
  return <div ref={ref} style={{ height: "100%", overflow: "auto" }} />;
}

export function ChangesModal() {
  const t = useT();
  useChangesRoute();
  const cwd = useTermStore((s) => s.changesCwd);
  const commit = useTermStore((s) => s.changesCommit);
  const range = useTermStore((s) => s.changesBase);
  const base = commit ? range?.from ?? null : null;
  const side = useTermStore((s) => s.changesSide) ?? "all";
  const initialPath = useTermStore((s) => s.changesPath);
  const close = useTermStore((s) => s.closeChanges);
  const [files, setFiles] = useState<ChangedFile[] | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [err, setErr] = useState("");
  const [tick, setTick] = useState(0);
  const [options, setOptionsState] = useState<DiffOptions>(readDiffOptions);
  const setOption = (key: keyof DiffOptions) => setOptionsState((current) => {
    const next = { ...current, [key]: !current[key] };
    writeDiffOptions(next);
    return next;
  });
  const dialog = useRef<HTMLDivElement>(null);

  useEffect(() => {
    setFiles(null);
    setSelected(null);
    setErr("");
  }, [cwd, commit, base, side]);

  useEffect(() => {
    if (!cwd) return;
    let alive = true;
    const load = commit && base ? gitRangeFiles(cwd, base, commit)
      : commit ? gitCommitFiles(cwd, commit) : gitChangedFiles(cwd).then((list) => list.files);
    void load.then((all) => {
      if (!alive) return;
      const fs = commit ? all : filesForSide(all, side);
      setFiles(fs);
      setSelected((cur) => {
        const wanted = cur ?? initialPath;
        return wanted && fs.some((f) => f.path === wanted) ? wanted : (fs[0]?.path ?? null);
      });
      setErr("");
    }).catch((e) => { if (alive) setErr(String(e)); });
    return () => { alive = false; };
  }, [cwd, commit, base, side, initialPath, tick]);

  useEffect(() => {
    if (!cwd) return;
    const previous = document.activeElement as HTMLElement | null;
    dialog.current?.focus();
    return () => { if (previous?.isConnected) previous.focus(); };
  }, [cwd]);

  // The URL names the file on screen, so a reload reopens it.
  useEffect(() => {
    if (cwd && selected) replaceChangesPath(selected);
  }, [cwd, selected]);

  if (!cwd) return null;
  const file = files?.find((f) => f.path === selected);
  const comparison = base && range ? t("changes.compareBranch", range.label) : commit ? t("changes.compareCommit") : side === "staged" ? t("changes.compareStaged")
    : side === "unstaged" || side === "untracked" ? t("changes.compareUnstaged") : t("changes.compareAll");
  const title = base ? t("git.branchChanges") : commit ? t("changes.commitTitle", commit.slice(0, 12)) : side === "staged" ? t("git.staged")
    : side === "unstaged" ? t("git.changes") : side === "untracked" ? t("git.untracked")
    : side === "conflicted" ? t("git.conflicts") : t("changes.title");

  return <Backdrop onClose={close} zIndex={1200}>
    <div className="changes-dialog" role="dialog" aria-modal="true" aria-label={title} tabIndex={-1} ref={dialog}
      onKeyDown={(e) => {
        if (e.key === "Escape") { e.preventDefault(); e.stopPropagation(); close(); }
        if (e.key !== "Tab") return;
        const nodes = Array.from(dialog.current?.querySelectorAll<HTMLElement>("button:not(:disabled), input, [tabindex='0']") ?? []);
        const first = nodes[0]; const last = nodes[nodes.length - 1];
        if (e.shiftKey && (document.activeElement === first || document.activeElement === dialog.current)) {
          e.preventDefault(); last?.focus();
        } else if (!e.shiftKey && document.activeElement === last) { e.preventDefault(); first?.focus(); }
      }}>
      <header className="changes-header">
        <div><strong>{title}</strong><div className="changes-comparison">{comparison}</div></div>
        <div className="changes-toolbar">
          <button type="button" className="vlx-btn changes-toggle" aria-pressed={options.ignoreWhitespace}
            onClick={() => setOption("ignoreWhitespace")}>{t("changes.ignoreWhitespace")}</button>
          <button type="button" className="vlx-btn changes-toggle" aria-pressed={options.collapseUnchanged}
            onClick={() => setOption("collapseUnchanged")}>{t("changes.collapseUnchanged")}</button>
          <button type="button" className="vlx-btn" onClick={() => setTick((n) => n + 1)}>{t("changes.refresh")}</button>
          <button type="button" className="vlx-btn" onClick={close}>{t("common.close")}</button>
        </div>
      </header>
      <div className="changes-body">
        <div className="changes-files" aria-label={t("git.changedFiles", files?.length ?? 0)}>
          {err && <div className="changes-message git-err" role="alert">{err}</div>}
          {!err && files === null && <div className="changes-message">{t("changes.loading")}</div>}
          {!err && files?.length === 0 && <div className="changes-message">{t("changes.noChanges")}</div>}
          {files?.map((f) => <div key={f.path} className={f.path === selected ? "changes-selected" : ""}>
            <GitFileRow file={f} additions={f.additions} deletions={f.deletions} binary={f.binary}
              onOpen={() => setSelected(f.path)} actions={[]} />
          </div>)}
        </div>
        <div className="changes-content">
          {selected ? <>
            <div className="changes-file-path">{file?.originalPath && <span>{file.originalPath} → </span>}{selected}</div>
            <div className="changes-editor"><DiffView key={`${cwd}:${base}:${commit}:${side}:${selected}`} cwd={cwd} path={selected} tick={tick} commit={commit} base={base} side={side} options={options} /></div>
          </> : <div className="changes-message">{t("changes.selectFile")}</div>}
        </div>
      </div>
    </div>
  </Backdrop>;
}

//! "Committed on this branch": the files the current branch changed since it split from a base
//! branch, with how far each side has moved. The working-tree groups above go empty once work is
//! committed; this is what a worktree session still has to merge or put up for review.
//!
//! The base is chosen per repository and remembered; until then the backend picks the remote's
//! default branch. On the base branch itself the backend has nothing to compare and the section
//! stays hidden.
//!
//! When the repository's `origin` is on the configured Gitea server, the section also opens a pull
//! request from this branch into the chosen base, pushing the branch first.

import { useEffect, useState } from "react";
import { FormModal } from "../../../components/FormModal";
import Select from "../../../components/Select";
import { useT } from "../../../i18n";
import {
  gitBranchCompare,
  gitOpenPullRequest,
  gitPrProvider,
  type BranchCompare,
  type GitPrProvider,
  type PrOutcome,
} from "../../../ipc/commands";
import { platform } from "../../../platform";
import { useTermStore } from "../../../store/termStore";
import { GitFileRow } from "./GitFileRow";
import { GitSection } from "./GitSection";

const BASE_KEY = "vlx.git.compareBase:";

function readBase(root: string): string | null {
  try {
    return localStorage.getItem(BASE_KEY + root);
  } catch {
    return null;
  }
}

function writeBase(root: string, base: string): void {
  try {
    localStorage.setItem(BASE_KEY + root, base);
  } catch {
    // The choice then lasts until the tab closes.
  }
}

export function BranchCompareSection({
  cwd,
  root,
  head,
  branch,
  refreshToken,
  open,
  onToggle,
}: {
  cwd: string;
  /** Checked-out branch, the head of a pull request; null when detached. */
  branch: string | null;
  /** Repository root, which keys the remembered base. */
  root: string;
  /** HEAD identity; a new commit reloads the comparison. */
  head: string | null;
  refreshToken: number;
  open: boolean;
  onToggle: () => void;
}) {
  const t = useT();
  const openChanges = useTermStore((s) => s.openChanges);
  const [base, setBase] = useState<string | null>(() => readBase(root));
  const [data, setData] = useState<BranchCompare | null>(null);
  const [err, setErr] = useState("");
  const [provider, setProvider] = useState<GitPrProvider | null>(null);
  const [prForm, setPrForm] = useState(false);
  const [opened, setOpened] = useState<PrOutcome | null>(null);

  useEffect(() => setBase(readBase(root)), [root]);

  // Asked once per repository: it may probe the Gitea server over the network.
  useEffect(() => {
    let alive = true;
    setProvider(null);
    setOpened(null);
    void gitPrProvider(cwd).then((p) => { if (alive) setProvider(p); }).catch(() => {});
    return () => {
      alive = false;
    };
  }, [cwd]);

  useEffect(() => {
    if (!head) {
      setData(null);
      return;
    }
    let alive = true;
    void gitBranchCompare(cwd, base)
      .then((result) => {
        if (!alive) return;
        setData(result);
        setErr("");
      })
      .catch((e) => {
        if (!alive) return;
        // A remembered base that no longer exists falls back to the default.
        if (base && /not found/i.test(String(e))) {
          setBase(null);
          return;
        }
        setErr(String(e));
      });
    return () => {
      alive = false;
    };
  }, [cwd, base, head, refreshToken]);

  if (!data && !err) return null;

  const openFile = (path?: string) => {
    if (!data) return;
    openChanges(cwd, { path, commit: data.head, base: { from: data.mergeBase, label: data.base } });
  };
  const choose = (next: string) => {
    writeBase(root, next);
    setBase(next);
  };

  return (
    <GitSection
      title={t("git.branchChanges")}
      count={data?.files.length ?? 0}
      open={open}
      onToggle={onToggle}
      view={data && data.files.length > 0 ? { label: t("git.viewAll"), onClick: () => openFile() } : undefined}
    >
      {err && <div className="insp-empty git-err">{err}</div>}
      {data && (
        <>
          <div className="git-compare-head">
            <Select
              value={data.base}
              onChange={choose}
              options={[...new Set([data.base, ...data.candidates])].map((name) => ({ value: name, label: name }))}
              width="100%"
              size="sm"
              leading={t("git.compareBase")}
              title={data.base}
              menuPortal
            />
            <span className="git-compare-counts" title={t("git.compareAheadBehind", data.ahead, data.behind, data.base)}>
              ↑{data.ahead} ↓{data.behind}
            </span>
          </div>
          {provider?.provider === "gitea" && branch && data.ahead > 0 && (
            <div className="git-pr">
              <button
                type="button"
                className="vlx-btn"
                disabled={!provider.available}
                title={provider.reason === "no_token" ? t("git.pr.noToken") : provider.reason === "unreachable" ? t("git.pr.unreachable") : undefined}
                onClick={() => { setOpened(null); setPrForm(true); }}
              >
                {t("git.pr.open")}
              </button>
              {!provider.available && (
                <span className="git-pr-note">{provider.reason === "no_token" ? t("git.pr.noToken") : t("git.pr.unreachable")}</span>
              )}
              {opened && (
                <span className="git-pr-note">
                  {opened.url
                    ? <a href={opened.url} onClick={(e) => { e.preventDefault(); void platform.opener.openExternal(opened.url); }}>{t("git.pr.opened")}</a>
                    : t("git.pr.opened")}
                </span>
              )}
            </div>
          )}
          {data.files.length === 0 && <div className="insp-empty">{t("git.noBranchChanges")}</div>}
          {data.files.map((f) => (
            <GitFileRow
              key={f.path}
              file={f}
              additions={f.additions}
              deletions={f.deletions}
              binary={f.binary}
              onOpen={() => openFile(f.path)}
              actions={[]}
            />
          ))}
        </>
      )}
      {prForm && data && branch && (
        <FormModal
          title={t("git.pr.formTitle")}
          description={t("git.pr.formDescription", branch, data.base)}
          fields={[
            { key: "title", label: t("git.pr.titleField"), autoFocus: true, placeholder: branch },
            {
              key: "body",
              label: t("git.pr.bodyField"),
              render: (value, onChange, context) => (
                <textarea className="vlx-input git-pr-body" rows={5} value={value} disabled={context.disabled}
                  onChange={(e) => onChange(e.target.value)} />
              ),
            },
          ]}
          initial={{ title: "", body: "" }}
          submitLabel={t("git.pr.open")}
          submittingLabel={t("git.pr.opening")}
          width={460}
          onSubmit={async (values) => {
            const outcome = await gitOpenPullRequest(cwd, data.base, values.title ?? "", values.body ?? "");
            setOpened(outcome);
            setPrForm(false);
          }}
          onCancel={() => setPrForm(false)}
        />
      )}
    </GitSection>
  );
}

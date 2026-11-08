//! Banner for a merge, rebase, cherry-pick, or revert that stopped on conflicts. It says which
//! operation is waiting, offers Continue once every conflict is resolved and staged, and Abort —
//! which throws away the resolutions made so far — behind a confirmation.

import { useState } from "react";
import { useT } from "../../../i18n";
import type { GitOperation } from "../../../types";

const TITLES = {
  merge: "git.operation.merge",
  rebase: "git.operation.rebase",
  "cherry-pick": "git.operation.cherry-pick",
  revert: "git.operation.revert",
} as const;

export function GitOperationBanner({
  operation,
  conflicts,
  disabled,
  onContinue,
  onAbort,
  agentLabel,
  onAskAgent,
}: {
  operation: GitOperation;
  /** Unresolved conflicted files; Continue waits until there are none. */
  conflicts: number;
  disabled: boolean;
  onContinue: () => void;
  onAbort: () => void;
  /** Agent of the shown session that can be asked to resolve the conflicts; null when there is none. */
  agentLabel?: string | null;
  onAskAgent?: () => void;
}) {
  const t = useT();
  const [confirmAbort, setConfirmAbort] = useState(false);
  const title = t(TITLES[operation]);

  return (
    <div className="git-op" role="status">
      <div className="git-op-title">{title}</div>
      <div className="git-op-text">
        {confirmAbort ? t("git.abortConfirm") : conflicts > 0 ? t("git.operationHint", conflicts) : t("git.operationReady")}
      </div>
      <div className="git-op-acts">
        {confirmAbort ? (
          <>
            <button
              type="button"
              className="vlx-btn git-danger-btn"
              disabled={disabled}
              onClick={() => {
                setConfirmAbort(false);
                onAbort();
              }}
            >
              {t("git.abort")}
            </button>
            <button type="button" className="vlx-btn" onClick={() => setConfirmAbort(false)}>
              {t("common.cancel")}
            </button>
          </>
        ) : (
          <>
            <button
              type="button"
              className="vlx-btn vlx-btn-primary"
              disabled={disabled || conflicts > 0}
              onClick={onContinue}
            >
              {t("git.continue")}
            </button>
            <button type="button" className="vlx-btn" disabled={disabled} onClick={() => setConfirmAbort(true)}>
              {t("git.abort")}
            </button>
            {agentLabel && onAskAgent && conflicts > 0 && (
              <button type="button" className="vlx-btn" onClick={onAskAgent}>
                {t("git.ask.resolve", agentLabel)}
              </button>
            )}
          </>
        )}
      </div>
    </div>
  );
}

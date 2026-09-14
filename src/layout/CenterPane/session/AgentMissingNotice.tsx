//! Inline guidance for a conversation whose agent executable cannot be started.
//!
//! The terminal view answers the same condition with its install card, but only a terminal has a shell
//! to run the recipe in. This notice therefore explains the state and hands installation over to the
//! terminal view, where the existing card performs it and takes care of detecting the new path.

import { useEffect, useState } from "react";

import { useT } from "../../../i18n";
import { agentInstallRecipe, type AgentInstallRecipe } from "../../../ipc/commands";
import { platform } from "../../../platform";
import type { Session } from "../../../types";
import { AGENT_KIND_LABEL, kindIconEl } from "../../sessionViewers/sessionMeta";

/** Missing-agent notice shown in the conversation when a send failed for lack of an executable. */
export function AgentMissingNotice({
  session,
  onInstall,
  onDismiss,
}: {
  session: Session;
  onInstall: () => void;
  onDismiss: () => void;
}) {
  const t = useT();
  const [recipe, setRecipe] = useState<AgentInstallRecipe | null>(null);

  useEffect(() => {
    let alive = true;
    void agentInstallRecipe(session.kind)
      .then((r) => {
        if (alive) setRecipe(r);
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [session.kind]);

  const label = recipe?.label ?? AGENT_KIND_LABEL[session.kind] ?? session.kind;
  const hasInstallCommand = !!recipe?.command.trim();
  const openDocs = () => {
    if (recipe) void platform.opener.openExternal(recipe.docsUrl).catch(() => {});
  };

  return (
    <div className="sv-agent-missing" role="status">
      <div className="sv-agent-missing-head">
        {kindIconEl(session.kind, 14)}
        <span className="sv-agent-missing-title">{t("agentInstall.title", label)}</span>
      </div>
      <div className="sv-agent-missing-desc">{t("agentInstall.desc", label)}</div>
      <div className="sv-agent-missing-actions">
        {hasInstallCommand && (
          <button className="vlx-btn vlx-btn-primary" onClick={onInstall}>
            {t("agentInstall.install")}
          </button>
        )}
        <button className="vlx-btn" onClick={openDocs} disabled={!recipe}>
          {t("agentInstall.docs")}
        </button>
        <span className="sv-agent-missing-spacer" />
        <button className="sv-agent-missing-link" onClick={onDismiss}>
          {t("agentInstall.dismiss")}
        </button>
      </div>
    </div>
  );
}

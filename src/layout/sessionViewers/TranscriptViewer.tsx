//! Agent transcript viewer for scrollable, searchable, copyable messages parsed by the backend.
//!
//! Two caller-selected modes:
//! - Archive browsing uses an internal query to filter messages and shows a message count.
//! - Global or archive search passes `scrollToMessageIndex`, displays every message, and scrolls to
//!   the target. It omits the internal header because the outer search console already owns search
//!   and match navigation. `initialQuery` controls highlighting; the target uses active styling and
//!   other matches use regular styling (see highlight.tsx).
//!
//! Messages are drawn with the session view's conversation components and stylesheet, so an archived
//! conversation reads like the live one. The text stays plain rather than Markdown so search literals
//! can be marked exactly as the index matched them.

import { useEffect, useMemo, useRef, useState } from "react";

import Icons from "../../components/Icons";
import { dateLocale, useT } from "../../i18n";
import type { TranscriptMessage } from "../../ipc/commands";
import type { Session } from "../../types";
import { MessageBubble } from "../CenterPane/session/rows";
import { highlightMatches } from "./highlight";
import { kindIconEl } from "./sessionMeta";
import "../CenterPane/session/session-view.css";
import "./transcript-viewer.css";

/** Assistant display name based on session type. */
export function assistantLabel(kind: Session["kind"]): string {
  if (kind === "codex") return "Codex";
  if (kind === "opencode") return "OpenCode";
  if (kind === "copilot") return "Copilot";
  if (kind === "cursor") return "Cursor";
  if (kind === "antigravity") return "Antigravity";
  if (kind === "cline") return "Cline";
  if (kind === "pi") return "Pi";
  if (kind === "omp") return "OMP";
  if (kind === "crush") return "Crush";
  if (kind === "kimi") return "Kimi Code (K3)";
  if (kind === "kiro") return "Kiro";
  if (kind === "grok") return "Grok Build (Grok 4.5)";
  if (kind === "zoo") return "Zoo Code";
  return "Claude";
}

/** Convert an ISO timestamp to local time, returning an empty string on parse failure. */
export function fmtTs(ts?: string | null): string {
  if (!ts) return "";
  const d = new Date(ts);
  return Number.isNaN(d.getTime()) ? "" : d.toLocaleString(dateLocale());
}

export function TranscriptViewer({
  session,
  messages,
  initialQuery,
  highlightTerms,
  scrollToMessageIndex,
}: {
  session: Session;
  messages: TranscriptMessage[];
  /** Initial browsing query; also the locate-mode highlight when `highlightTerms` is absent. */
  initialQuery?: string;
  /** Locate mode: literals the search index matched, highlighted instead of the raw query. */
  highlightTerms?: string[];
  /** Message index to scroll to and highlight; providing it enables locate mode. */
  scrollToMessageIndex?: number;
}) {
  const t = useT();
  const label = assistantLabel(session.kind);
  const isLocate = scrollToMessageIndex != null;
  const targetRef = useRef<HTMLDivElement | null>(null);

  // Internal query for browsing mode; locate mode is controlled by highlightTerms/initialQuery.
  const [browseQuery, setBrowseQuery] = useState(initialQuery ?? "");
  // Effective highlight terms: the matched literals in locate mode, the internal input otherwise.
  const terms: string[] = isLocate
    ? (highlightTerms ?? [initialQuery ?? ""]).map((s) => s.trim()).filter(Boolean)
    : [browseQuery.trim()].filter(Boolean);

  // Locate mode preserves all messages and original indices; browsing mode filters by its query.
  const visible = useMemo(() => {
    const all = messages.map((m, idx) => ({ m, idx }));
    if (isLocate) return all;
    const ql = browseQuery.trim().toLowerCase();
    if (!ql) return all;
    return all.filter(({ m }) => m.text.toLowerCase().includes(ql));
  }, [messages, browseQuery, isLocate]);

  // Center the target message after mounting or changing the locate target.
  useEffect(() => {
    if (!isLocate) return;
    targetRef.current?.scrollIntoView({ block: "center" });
  }, [isLocate, scrollToMessageIndex, messages]);

  return (
    <div className="sv">
      {/* Browsing mode shows the query and message count. Locate mode delegates search and navigation to its outer console. */}
      {!isLocate && (
        <div className="searchbar tv-searchbar">
          <div className="box">
            <Icons.search size={13} />
            <input
              placeholder={t("archive.searchTranscript")}
              value={browseQuery}
              onChange={(e) => setBrowseQuery(e.target.value)}
            />
          </div>
          <span className="tv-count">
            {browseQuery.trim()
              ? t("archive.msgCountFiltered", visible.length, messages.length)
              : t("archive.msgCountAll", messages.length)}
          </span>
        </div>
      )}

      <div className="sv-scroll">
        {visible.map(({ m, idx }) => {
          const isUser = m.role === "user";
          const isTarget = isLocate && idx === scrollToMessageIndex;
          const origin = m.origin;
          const who = isUser
            ? origin
              ? `${origin.name}${origin.role === "plan" || origin.role === "exec" ? ` · ${t(origin.role === "plan" ? "chat.origin.plan" : "chat.origin.exec")}` : ""}`
              : t("archive.you")
            : label;
          return (
            <div key={idx} ref={isTarget ? targetRef : undefined} className={isTarget ? "tv-target" : undefined}>
              <MessageBubble
                who={who}
                icon={isUser ? (origin ? kindIconEl(origin.agent, 14) : undefined) : kindIconEl(session.kind, 14)}
                isUser={isUser}
                at={m.timestamp ?? undefined}
                text={m.text ?? ""}
                renderText={(text) => (
                  <div className="tv-text">
                    {terms.length > 0 ? highlightMatches(text, terms, isTarget) : text}
                  </div>
                )}
                footnote={m.tools.length > 0 ? <div className="tv-tools">{t("archive.toolsUsed", m.tools.join(" · "))}</div> : undefined}
              />
            </div>
          );
        })}
        {visible.length === 0 && (
          <div className="sv-empty">
            {browseQuery.trim() ? t("archive.noMatch") : t("archive.emptyTranscript")}
          </div>
        )}
      </div>
    </div>
  );
}

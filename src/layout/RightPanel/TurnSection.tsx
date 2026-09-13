//! Turn accounting is owned by the backend. This component only formats the current snapshot.
import { useEffect, useRef, useState } from "react";
import { fmtTokens } from "../../format";
import { agentTurnStats, type AgentTurnStats } from "../../ipc/commands";
import type { Session } from "../../types";
import { Section } from "./parts";
import "./turn-section.css";

const tokens = (value: number | null | undefined) => value == null ? "—" : fmtTokens(value);
const contextTokens = (value: number | null | undefined) => value != null && value >= 1000 && value < 1_000_000
  ? `${(value / 1000).toFixed(1)}k` : tokens(value);

export function TurnStatsView({ stats, state = "ready" }: {
  stats: AgentTurnStats | null;
  state?: "loading" | "waiting" | "error" | "ready";
}) {
  const percent = stats?.contextPercent;
  const tone = percent != null && percent >= 90 ? "crit" : percent != null && percent >= 70 ? "warn" : "ok";
  const context = `${contextTokens(stats?.contextTokens)} / ${contextTokens(stats?.contextLimit)}`;
  const rate = stats?.generationTokensPerSecond;
  const model = stats?.model;
  // Each group renders only when it has something to say, so an agent that reports nothing shows the
  // note alone instead of a column of dashes.
  const hasContext = stats?.contextTokens != null || stats?.contextLimit != null || percent != null;
  const hasTurn = stats?.inputTokens != null || stats?.tokens != null || stats?.totalTokens != null
    || stats?.cacheHitPercent != null || rate != null || stats?.filesTouched != null;
  const hasSession = stats?.sessionTokens != null;
  const hasData = model != null || hasContext || hasTurn || hasSession;
  return <Section id="turn" title="This turn" className="turn-section" ariaLabel="This turn" busy={state === "loading"}>
    {model != null && (
      <div className="kv" title="Model named by the most recent context reading from this session.">
        <span className="k">model</span><span className="v">{model}</span>
      </div>
    )}
    {hasContext && (
      <div className={`turn-context ${tone}`}
        title="Tokens currently in the model context / context capacity. This is not cumulative token usage.">
        <div className="kv">
          <span className="k">context</span>
          <span className="v">
            {context}
            {percent != null && <span className="turn-pill">{Math.round(percent)}%</span>}
          </span>
        </div>
        {percent != null && (
          <div className="turn-meter" role="progressbar"
            aria-label="Context usage" aria-valuemin={0} aria-valuemax={100}
            aria-valuenow={Math.min(100, Math.max(0, percent))}
            aria-valuetext={context}>
            <i style={{ width: `${Math.min(100, Math.max(0, percent))}%` }} />
          </div>
        )}
      </div>
    )}
    {hasTurn && (
      <div className="turn-rows">
        <div className="turn-divider" />
        <div className="turn-metrics">
          <div className="turn-metric" title="Input plus output across model requests in this turn. Repeated context counts each time; cached input is already included.">
            <span className="m-k">turn tokens</span><span className="m-v">{tokens(stats?.totalTokens)}</span>
          </div>
          <div className="turn-metric" title="Input tokens, including cache reads and writes.">
            <span className="m-k">input</span><span className="m-v">{tokens(stats?.inputTokens)}</span>
          </div>
          <div className="turn-metric" title="Generated output tokens, including reasoning when reported by the agent.">
            <span className="m-k">output</span><span className="m-v">{tokens(stats?.tokens)}</span>
          </div>
          <div className="turn-metric" title={stats?.cachedTokens == null ? "Cached input divided by total input for this turn." : `${stats.cachedTokens.toLocaleString()} cached input tokens. Cache reads / total input for this turn.`}>
            <span className="m-k">cache hit</span>
            <span className="m-v">{stats?.cacheHitPercent == null ? "—" : `${stats.cacheHitPercent.toFixed(1)}%`}</span>
          </div>
          <div className="turn-metric" title="Average rate of completed, measured model streams, including reported reasoning tokens. Excludes tool execution and user waits. Unavailable when stream timing was not recorded.">
            <span className="m-k">generation</span>
            <span className="m-v">{rate == null ? "—" : `${rate.toFixed(1)} tok/s`}</span>
          </div>
          <div className="turn-metric" title="Distinct paths from successful, explicitly recorded file edits. Shell edits and child-agent changes may not be reported.">
            <span className="m-k">files changed</span><span className="m-v">{stats?.filesTouched == null ? "—" : String(stats.filesTouched)}</span>
          </div>
        </div>
      </div>
    )}
    {hasSession && (
      <div className="turn-rows" title="Cumulative input plus output for this recorded main session. Cached input is included; independent child-agent sessions are excluded.">
        <div className="turn-divider" />
        <div className="kv">
          <span className="k">session total</span>
          <span className="v turn-strong">{tokens(stats?.sessionTokens)}<small>tokens</small></span>
        </div>
      </div>
    )}
    {!hasData && <p className="turn-status" role="status">{
      state === "loading" ? "Loading statistics…" : state === "waiting" ? "Statistics appear after the first turn." :
      state === "error" ? "Statistics are temporarily unavailable." : "This agent has not reported usage statistics."
    }</p>}
  </Section>;
}

export function TurnSection({ session, agentState, currentTool }: {
  session: Session; agentState?: string | null; currentTool?: string | null;
}) {
  const [stats, setStats] = useState<AgentTurnStats | null>(null);
  const [state, setState] = useState<"loading" | "waiting" | "error" | "ready">("loading");
  const previous = useRef({ identity: "", agentState });
  useEffect(() => {
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const identity = `${session.id}:${session.agentSessionId ?? ""}`;
    const reset = previous.current.identity !== identity || (agentState === "working" && previous.current.agentState !== "working");
    previous.current = { identity, agentState };
    if (reset) { setStats(null); setState("loading"); }
    if (!session.agentSessionId) { setState("waiting"); return; }
    const sample = async (settled = false) => {
      try {
        const next = await agentTurnStats(session.id);
        if (!cancelled) { setStats(next); setState("ready"); }
      } catch {
        if (!cancelled) { setStats(null); setState("error"); }
      }
      // Serialize reads. A final delayed read absorbs the provider's turn-end disk flush.
      if (!cancelled && (agentState === "working" || !settled)) {
        timer = setTimeout(() => { void sample(true); }, agentState === "working" ? 1500 : 750);
      }
    };
    void sample();
    return () => { cancelled = true; clearTimeout(timer); };
  }, [session.id, session.agentSessionId, agentState, currentTool]);
  return <TurnStatsView stats={stats} state={state} />;
}

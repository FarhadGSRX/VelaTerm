//! Two independent launch drafts backed by the same agent capability catalogue.
import { useEffect, useState } from "react";
import { useT } from "../i18n";
import type { SpawnRequest } from "../ipc/events";
import { planExecuteDefaults, type LaunchOption } from "../ipc/launch";
import type { PlanExecuteRolePrefs } from "../store/settings";
import { useTermStore } from "../store/termStore";
import type { SessionKind } from "../types";
import Combo from "./Combo";
import { ModelEffortFields, LaunchField, LaunchLoadState, launchErrorText } from "./LaunchFields";

type Config = NonNullable<SpawnRequest["planExecute"]>;
type Role = "plan" | "exec";

function launchable(value: Config, options: LaunchOption[]): Config | null {
  return (["plan", "exec"] as const).every(role =>
    options.some(option => option.id === value[role].agent && option.supportsPlanExecute)) ? value : null;
}

/**
 * Restores the remembered choices for fields the request leaves open. A remembered agent that differs
 * from the backend's default also drops the backend's model and effort: those belonged to another agent.
 */
function withRemembered(
  value: Config,
  explicit: Config | null,
  prefs: { plan: PlanExecuteRolePrefs; exec: PlanExecuteRolePrefs },
  options: LaunchOption[],
): Config {
  const merge = (role: Role) => {
    const base = { ...value[role] };
    const given = explicit?.[role] ?? {};
    const saved = prefs?.[role] ?? {};
    // Old preferences must not restore an agent that can no longer run this workflow.
    if (saved.agent && !options.some(option => option.id === saved.agent && option.supportsPlanExecute)) return base;
    if (given.agent == null && saved.agent) {
      if (base.agent !== saved.agent) {
        base.model = "";
        base.effort = "";
      }
      base.agent = saved.agent;
    }
    if (given.model == null && saved.model !== undefined) base.model = saved.model;
    if (given.effort == null && saved.effort !== undefined) base.effort = saved.effort;
    return base;
  };
  return { ...value, plan: merge("plan"), exec: merge("exec") };
}

export function PlanExecuteFields({ parentSessionId, config, options, onChange, resolved = false, cwd }: {
  parentSessionId: string; config: Config; options: LaunchOption[];
  onChange: (value: Config | null) => void;
  resolved?: boolean;
  cwd?: string | null;
}) {
  const t = useT();
  const [draft, setDraft] = useState<Config | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [revision, setRevision] = useState(0);
  useEffect(() => {
    let live = true;
    setDraft(null); setError(null); onChange(null);
    void (resolved ? Promise.resolve(config) : planExecuteDefaults(parentSessionId, config)).then(value => {
      if (!live) return;
      // Read the memory once, when the draft is resolved: a choice remembered from this very dialog must
      // not restart the load and wipe the other panel's edits.
      const seeded = withRemembered(value, resolved ? null : config, useTermStore.getState().planExecutePrefs, options);
      setDraft(seeded); onChange(launchable(seeded, options));
    }).catch((cause) => { if (live) setError(launchErrorText(cause)); });
    return () => { live = false; };
  }, [parentSessionId, config, revision, onChange, resolved, options]);
  const publish = (next: Config) => { setDraft(next); onChange(launchable(next, options)); };
  const available = options.filter(option => option.supportsPlanExecute);
  if (!draft) return <LaunchLoadState state={error ? "error" : "loading"} error={error} retry={() => setRevision(r => r + 1)} />;
  return <div className="launch-role-panels">
    <label className="launch-panel launch-split-option">
      <span className="launch-inline"><input type="checkbox" checked={draft.splitTasks ?? false}
        onChange={event => publish({ ...draft, splitTasks: event.target.checked })} />
        <strong>{t("launch.splitTasks")}</strong></span>
      <span className="launch-hint">{t("launch.splitTasksHint")}</span>
    </label>
    {(["plan", "exec"] as const).map(role => {
      const value = draft[role];
      const spec = available.find(option => option.id === value.agent);
      const update = (patch: Partial<typeof value>) => {
        const next = { ...draft, [role]: { ...value, ...patch } };
        publish(next);
      };
      const remember = (patch: { agent?: SessionKind | null; model?: string | null; effort?: string | null }) =>
        useTermStore.getState().setPlanExecuteRolePrefs(role, patch);
      const label = t(role === "plan" ? "launch.planTitle" : "launch.execTitle");
      return <section className="launch-panel" key={role} aria-label={label}>
        <h3>{label}</h3>
        <LaunchField label={t("spawn.agentLabel")}>
          <Combo value={value.agent ?? ""} width="100%" menuPortal ariaLabel={t("spawn.agentLabel")}
            options={available.map(option => ({ value: option.id, label: option.label }))}
            onChange={agent => {
              update({ agent: agent as typeof value.agent, model: "", effort: "" });
              remember({ agent: agent as SessionKind, model: null, effort: null });
            }} />
        </LaunchField>
        {spec && <div className="launch-model-grid">
          <ModelEffortFields spec={spec} model={value.model ?? ""} effort={value.effort ?? ""}
            context={{ parentSessionId, cwd }} onChange={(model, effort) => update({ model, effort })}
            onModelCommit={model => remember({ model })} onEffortCommit={effort => remember({ effort })} />
        </div>}
      </section>;
    })}
  </div>;
}

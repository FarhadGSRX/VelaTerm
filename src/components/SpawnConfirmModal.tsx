//! Review one child-session request without taking focus away from the current session.

import { useEffect, useId, useMemo, useRef, useState } from "react";
import { useT } from "../i18n";
import { useSuspendNativeViews } from "../hooks/nativeViewSuspend";
import { applyLaunchArgs, createPlanExecute, launchSelection, preparePlanExecute, type PlanExecutePreparation } from "../ipc/launch";
import type { SpawnRequest, WorktreeMode } from "../ipc/events";
import { useTermStore } from "../store/termStore";
import {
  launchErrorText,
  ModelEffortFields,
  LaunchField,
  LaunchLoadState,
  useLaunchOptions,
  WorktreeChoices,
} from "./LaunchFields";
import Combo from "./Combo";
import { useTaskImages } from "./useTaskImages";
import { dataUrl } from "../layout/CenterPane/session/attachments";
import { Icons } from "./Icons";
import { PlanExecuteFields } from "./PlanExecuteFields";
import { navigatePlanExecute, planExecuteUrl, usePlanExecuteMenuRoute } from "./planExecuteNavigation";

export function SpawnConfirmModal() {
  const t = useT();
  const titleId = useId();
  const queue = useTermStore((s) => s.pendingSpawns);
  const sessions = useTermStore((s) => s.sessions);
  const projects = useTermStore((s) => s.projects);
  const agentDefaults = useTermStore((s) => s.agentDefaults);
  const confirmSpawn = useTermStore((s) => s.confirmSpawn);
  const cancelSpawn = useTermStore((s) => s.cancelSpawn);
  const menu = usePlanExecuteMenuRoute();
  const menuRequest = useMemo<SpawnRequest | null>(() => menu ? {
    requestId: menu.requestId, parentSessionId: menu.context.parentSessionId ?? "", prompt: "",
    worktree: false, planExecute: { plan: {}, exec: {} },
  } : null, [menu]);
  const req = menuRequest ?? queue[0];
  const taskImages = useTaskImages(req, req?.images);
  const [preparation, setPreparation] = useState<PlanExecutePreparation | null>(null);
  const [preparationError, setPreparationError] = useState<string | null>(null);
  const [preparationRevision, setPreparationRevision] = useState(0);
  const [directory, setDirectory] = useState("");
  useEffect(() => {
    if (!menu) return;
    let live = true;
    setPreparation(null); setPreparationError(null);
    void preparePlanExecute(menu.context).then(value => {
      if (live) { setPreparation(value); setDirectory(value.cwd ?? ""); setWorktree(value.config.worktreeMode ?? (value.worktree ? "shared" : "none")); }
    }).catch(cause => { if (live) setPreparationError(launchErrorText(cause)); });
    return () => { live = false; };
  }, [menu, preparationRevision]);
  const parent = sessions.find((s) => s.id === req?.parentSessionId);
  const catalog = useLaunchOptions(Boolean(req));
  const requestedKind =
    req?.kind ||
    (parent?.kind !== "terminal" && parent?.kind !== "browser"
      ? parent?.kind
      : undefined) ||
    "claude";
  const [prompt, setPrompt] = useState("");
  const [kind, setKind] = useState<string>(requestedKind);
  const [worktree, setWorktree] = useState<WorktreeMode>("none");
  const [model, setModel] = useState("");
  const [effort, setEffort] = useState("");
  const [selectionReady, setSelectionReady] = useState(false);
  const [selectionError, setSelectionError] = useState<string | null>(null);
  const [revision, setRevision] = useState(0);
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const errorElement = useRef<HTMLParagraphElement | null>(null);
  useEffect(() => { if (error) errorElement.current?.scrollIntoView?.({ block: "nearest" }); }, [error]);
  const draftRequest = useRef<SpawnRequest | undefined>(undefined);
  const [workflow, setWorkflow] = useState<SpawnRequest["planExecute"]>(null);
  useEffect(() => {
    if (!req) return;
    if (draftRequest.current === req || (req.requestId && draftRequest.current?.requestId === req.requestId)) return;
    draftRequest.current = req;
    setPrompt(req.prompt);
    setKind(requestedKind);
    setWorktree(req.planExecute?.worktreeMode ?? (req.worktree !== false ? (req.planExecute ? "shared" : "each") : "none"));
    setError(null);
    setSubmitting(false);
  }, [req, requestedKind]);
  useEffect(() => {
    if (!req || req.planExecute) return;
    let live = true;
    setSelectionReady(false);
    setSelectionError(null);
    const inherited = parent?.kind === kind ? parent.agentArgs : null;
    const args = inherited || agentDefaults[kind]?.args || null;
    void launchSelection(kind, args)
      .then((selection) => {
        if (!live) return;
        setModel(
          kind === requestedKind
            ? (req.model ?? selection.model)
            : selection.model,
        );
        setEffort(
          kind === requestedKind
            ? (req.effort ?? selection.effort)
            : selection.effort,
        );
        setSelectionReady(true);
      })
      .catch(cause => {
        if (live) setSelectionError(launchErrorText(cause));
      });
    return () => {
      live = false;
    };
    // Only a new request or user-selected agent resets the draft, not background session updates.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [req, kind, revision]);
  useSuspendNativeViews(Boolean(req));
  if (!req) return null;
  const spec = catalog.options.find((option) => option.id === kind);
  const cwd =
    (menu ? directory : req.cwd) ||
    parent?.cwd ||
    projects?.find((p) => p.id === parent?.projectId)?.rootPath;
  const canLaunch =
    Boolean(prompt.trim()) &&
    (!menu || (!!preparation && Boolean(directory.trim()))) &&
    catalog.state === "ready" &&
    (req.planExecute ? !!workflow : selectionReady && !!spec) &&
    !taskImages.busy &&
    (!taskImages.attachments.length || !!req.planExecute || !!spec?.acceptsTask) &&
    !submitting;
  const launch = async () => {
    if (!canLaunch) return;
    setSubmitting(true);
    setError(null);
    try {
      if (req.planExecute && workflow) {
        for (const role of ["plan", "exec"] as const) {
          const choice = workflow[role];
          try { await applyLaunchArgs(choice.agent!, null, choice.model, choice.effort); }
          catch (cause) { throw new Error(`${t(role === "plan" ? "launch.planTitle" : "launch.execTitle")}: ${launchErrorText(cause)}`); }
        }
      }
      const images = taskImages.attachments.map(({ mimeType, data }) => ({ mimeType, data }));
      const imageFields = images.length || req.images ? { images } : {};
      if (menu && workflow) {
        const result = await createPlanExecute({requestId: menu.requestId, context: menu.context,
          prompt, cwd: directory.trim() || null, worktree: worktree !== "none", config: { ...workflow, worktreeMode: worktree }, ...imageFields});
        await useTermStore.getState().loadTree();
        if (result.run.state === "blocked") throw new Error(result.run.summary);
        navigatePlanExecute(planExecuteUrl(null));
        useTermStore.getState().openSession(result.planner.id);
        return;
      }
      if (req.planExecute && workflow) {
        await confirmSpawn({ ...req, prompt, worktree: worktree !== "none", planExecute: { ...workflow, worktreeMode: worktree }, ...imageFields });
        return;
      }
      await applyLaunchArgs(
        kind,
        null,
        spec?.acceptsTask ? model.trim() : null,
        spec?.effortFlag ? effort.trim() : null,
      );
      await confirmSpawn({
        ...req,
        ...imageFields,
        prompt,
        kind: kind as SpawnRequest["kind"],
        worktree: worktree !== "none",
        model: spec?.acceptsTask ? model.trim() : null,
        effort: spec?.effortFlag ? effort.trim() : null,
      });
    } catch (cause) {
      setError(launchErrorText(cause));
      setSubmitting(false);
    }
  };
  return (
    <section
      className="launch-dialog launch-single"
      role="dialog"
      aria-labelledby={titleId}
      onKeyDown={(e) => {
        if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
          e.preventDefault();
          void launch();
        }
      }}
    >
      <header className="launch-header">
        <div>
          <h2 id={titleId}>{t(menu ? "tree.newPlanExecuteSession" : "spawn.title")}</h2>
          <p>{t(req.planExecute ? "launch.planExecuteIntro" : "launch.singleIntro")}</p>
        </div>
        {!menu && queue.length > 1 && (
          <span className="launch-badge">
            {t("spawn.remaining", queue.length - 1)}
          </span>
        )}
      </header>
      <div className="launch-body">
        <div className="launch-context">
          <span>{t(menu ? "launch.createIn" : "spawn.fromSession")}</span>
          <strong>{menu ? preparation?.locationNames.join(" / ") : (parent?.name || t("common.session"))}</strong>
          {!menu && cwd && <code title={cwd}>{cwd}</code>}
        </div>
        {menu && !preparation && <LaunchLoadState state={preparationError ? "error" : "loading"} error={preparationError} retry={() => setPreparationRevision(r => r + 1)} />}
        {!req.planExecute && spec?.acceptsTask === false ? (
          <p className="launch-notice">{t("launch.terminalHint")}</p>
        ) : (
          <LaunchField
            label={t("spawn.promptLabel")}
            hint={t(menu ? "launch.planExecuteTaskHint" : "launch.taskHint")}
          >
            <textarea
              className="vlx-input launch-task"
              rows={6}
              autoFocus={!!menu}
              onPaste={taskImages.onPaste}
              disabled={submitting}
              value={prompt}
              onChange={(e) => setPrompt(e.target.value)}
            />
          </LaunchField>
        )}
        {taskImages.attachments.length > 0 && <div className="launch-images">
          {taskImages.attachments.map(image => <div className="launch-image" key={image.id} title={image.name}>
            <img src={dataUrl(image)} alt={image.name} />
            <button type="button" className="launch-image-remove" title={t("chat.attach.remove")} disabled={submitting || taskImages.busy} onClick={() => taskImages.remove(image.id)}>
              <Icons.close size={12} />
            </button>
          </div>)}
        </div>}
        {taskImages.busy && <p className="launch-hint" role="status">{t("common.loading")}</p>}
        {taskImages.note && <p className="launch-error" role="alert">{taskImages.note}</p>}
        {req.planExecute ? <>
          <LaunchLoadState state={catalog.state} error={catalog.error} retry={catalog.retry} />
          {catalog.state === "ready" && (!menu || preparation) && <PlanExecuteFields parentSessionId={req.parentSessionId}
            config={preparation && menu ? preparation.config : req.planExecute} resolved={!!menu} cwd={cwd} options={catalog.options} onChange={setWorkflow} />}
        </> : <section className="launch-panel">
          <h3>{t("launch.runtime")}</h3>
          <LaunchLoadState state={catalog.state} error={catalog.error} retry={catalog.retry} />
          {catalog.state === "ready" && (
            <>
              <LaunchField label={t("spawn.agentLabel")}>
                <Combo
                  value={kind}
                  onChange={setKind}
                  width="100%"
                  menuPortal
                  ariaLabel={t("spawn.agentLabel")}
                  options={catalog.options.map((o) => ({
                    value: o.id,
                    label: o.id === "terminal" ? t("kind.terminal") : o.label,
                  }))}
                />
              </LaunchField>
              {!selectionReady && (
                <LaunchLoadState
                  state={selectionError ? "error" : "loading"}
                  error={selectionError}
                  retry={() => setRevision((r) => r + 1)}
                />
              )}
              {selectionReady && spec?.acceptsTask && (
                <div className="launch-model-grid">
                  <ModelEffortFields spec={spec} model={model} effort={effort}
                    context={{ parentSessionId: req.parentSessionId, cwd, inheritArgs: true }}
                    onChange={(nextModel, nextEffort) => { setModel(nextModel); setEffort(nextEffort); }} />
                </div>
              )}
            </>
          )}
        </section>}
        {menu && <LaunchField label={t("launch.workingDirectory")}>
          <input className="vlx-input" value={directory} disabled={!preparation} placeholder="/path/to/project" onChange={e => setDirectory(e.target.value)} />
        </LaunchField>}
        <WorktreeChoices
          single={!req.planExecute}
          workflow={!!req.planExecute}
          value={worktree}
          onChange={setWorktree}
        />
        {error && (
          <p ref={errorElement} className="launch-error" role="alert">
            {t("launch.startError")} <span className="launch-error-detail">{error}</span>
          </p>
        )}
      </div>
      <footer className="launch-footer">
        <span className="launch-hint">{t(req.planExecute ? "launch.planExecuteResult" : "launch.singleResult")}</span>
        <div className="launch-actions">
          <button
            type="button"
            className="vlx-btn"
            onClick={() => menu ? navigatePlanExecute(planExecuteUrl(null)) : cancelSpawn()}
            disabled={submitting}
          >
            {t("common.cancel")}
          </button>
          <button
            type="button"
            className="vlx-btn vlx-btn-primary"
            disabled={!canLaunch}
            onClick={() => void launch()}
          >
            {t(submitting ? "launch.starting" : menu ? "launch.createAndStart" : "spawn.launch")}
          </button>
        </div>
      </footer>
    </section>
  );
}

//! Building blocks shared by the artifact tab and the panel beside a conversation: header controls, the
//! sandboxed frame on its stage, the console drawer, and the blocked-network and file-gone states.

import { useLayoutEffect, useRef, useState } from "react";

import Icons from "../../../../components/Icons";
import { useT } from "../../../../i18n";
import { isTauri } from "../../../../ipc/transport";
import { env, platform } from "../../../../platform";
import { useTermStore } from "../../../../store/termStore";
import type { ArtifactViewport } from "../../../../store/docTab";
import { consoleReport } from "./artifactBridge";
import { frameLayout, VIEWPORT_ORDER, viewportReadout, type ViewportSize } from "./artifactViewport";
import { insertIntoComposer } from "./artifactWriters";
import type { ArtifactState } from "./useArtifact";
import "./artifact.css";

/** Sandbox flags: scripts, forms and dialogs run; same-origin access, popups and top navigation do not. */
const SANDBOX = "allow-scripts allow-forms allow-modals";

const VIEWPORT_ICON = { fit: Icons.fit, desktop: Icons.monitor, tablet: Icons.tablet, phone: Icons.phone } as const;
const VIEWPORT_LABEL = {
  fit: "artifact.viewportFit",
  desktop: "artifact.viewportDesktop",
  tablet: "artifact.viewportTablet",
  phone: "artifact.viewportPhone",
} as const;

/** Chips and buttons for a header: writer, live, errors, network, viewport, reload, open externally. */
export function ArtifactHeaderControls({
  state,
  path,
  viewport,
  onViewport,
  consoleOpen,
  onToggleConsole,
  compact = false,
}: {
  state: ArtifactState;
  path: string;
  viewport?: ArtifactViewport;
  onViewport?: (v: ArtifactViewport) => void;
  consoleOpen: boolean;
  onToggleConsole: () => void;
  /** Beside a conversation: the writer is the conversation itself, and the panel has little room. */
  compact?: boolean;
}) {
  const t = useT();
  const writerName = useTermStore((s) => (state.writer ? s.sessions.find((x) => x.id === state.writer)?.name : undefined));
  const canOpenExternally = (isTauri || env.isElectron) && !compact;
  return (
    <>
      {!compact && state.writer && writerName && (
        <button
          className="artifact-chip writer"
          title={t("artifact.goToSession")}
          onClick={() => useTermStore.getState().openSession(state.writer!)}
        >
          <Icons.bot size={12} />
          <span>{writerName}</span>
        </button>
      )}
      {state.live && <span className="artifact-chip live" title={t("artifact.liveTitle")}>{t("artifact.live")}</span>}
      {state.errorCount + state.warnCount > 0 && (
        <button className={"artifact-chip" + (state.errorCount ? " err" : "")} aria-pressed={consoleOpen} onClick={onToggleConsole}>
          {state.errorCount ? <Icons.warn size={12} /> : null}
          {state.errorCount ? t("artifact.errorCount", state.errorCount) : t("artifact.warningCount", state.warnCount)}
        </button>
      )}
      <button
        className={"artifact-chip" + (state.net ? " online" : "")}
        title={t(state.net ? "artifact.onlineTitle" : "artifact.offlineTitle")}
        aria-pressed={state.net}
        onClick={() => state.setNet(!state.net)}
      >
        {state.net ? <Icons.globe size={12} /> : <Icons.shield size={12} />}
        {t(state.net ? "artifact.online" : "artifact.offline")}
      </button>
      {viewport && onViewport && (
        <div className="docview-seg icons" role="group">
          {VIEWPORT_ORDER.map((v) => {
            const Icon = VIEWPORT_ICON[v];
            return (
              <button
                key={v}
                className={viewport === v ? "on" : ""}
                aria-pressed={viewport === v}
                title={t(VIEWPORT_LABEL[v])}
                aria-label={t(VIEWPORT_LABEL[v])}
                onClick={() => onViewport(v)}
              >
                <Icon size={13} />
              </button>
            );
          })}
        </div>
      )}
      <button className="artifact-icon-btn" title={t("artifact.reload")} aria-label={t("artifact.reload")} onClick={state.reload}>
        <Icons.restart size={14} />
      </button>
      {canOpenExternally && (
        <button
          className="artifact-icon-btn"
          title={t("artifact.openInBrowser")}
          aria-label={t("artifact.openInBrowser")}
          onClick={() => void platform.opener.openPath(path)}
        >
          <Icons.external size={14} />
        </button>
      )}
    </>
  );
}

/** The frame on its stage, sized for the viewport; also the banner and console that belong under a header. */
export function ArtifactPane({
  state,
  path,
  title,
  viewport = "fit",
  consoleOpen,
  onCloseConsole,
  onCloseTab,
}: {
  state: ArtifactState;
  path: string;
  title: string;
  viewport?: ArtifactViewport;
  consoleOpen: boolean;
  onCloseConsole: () => void;
  onCloseTab?: () => void;
}) {
  const t = useT();
  if (state.missing) {
    return (
      <div className="artifact-pane">
        <div className="artifact-empty">
          <Icons.file size={20} />
          <span className="artifact-empty-title">{t("artifact.fileGone")}</span>
          <span className="artifact-empty-body">{t("artifact.fileGoneBody", title)}</span>
          {onCloseTab && (
            <button className="vlx-btn" onClick={onCloseTab}>
              {t("doc.closeTab")}
            </button>
          )}
        </div>
      </div>
    );
  }
  return (
    <div className="artifact-pane">
      {!state.net && state.blocked > 0 && (
        <div className="docview-banner artifact-banner-warn" role="status">
          <Icons.shield size={13} />
          <span style={{ flex: 1 }}>{t("artifact.netBlocked", state.blocked)}</span>
          <button onClick={() => state.setNet(true)}>{t("artifact.allowForFile")}</button>
        </div>
      )}
      <ArtifactStage state={state} viewport={viewport} title={title} />
      {consoleOpen && <ArtifactConsole state={state} path={path} onClose={onCloseConsole} />}
    </div>
  );
}

function ArtifactStage({ state, viewport, title }: { state: ArtifactState; viewport: ArtifactViewport; title: string }) {
  const t = useT();
  const ref = useRef<HTMLDivElement | null>(null);
  const [area, setArea] = useState<ViewportSize>({ width: 0, height: 0 });
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const measure = () => setArea({ width: el.clientWidth, height: el.clientHeight });
    measure();
    if (typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, []);
  const layout = frameLayout(viewport, area);
  const readout = viewportReadout(viewport, layout);
  const frame = state.url && (
    <iframe
      ref={state.frameRef}
      src={state.url}
      sandbox={SANDBOX}
      referrerPolicy="no-referrer"
      title={t("artifact.frameTitle", title)}
      style={viewport === "fit" ? undefined : { width: layout.width, height: layout.height, transform: `scale(${layout.scale})` }}
    />
  );
  return (
    <div ref={ref} className={"artifact-stage" + (viewport === "fit" ? " fit" : "")}>
      {viewport === "fit" ? (
        frame
      ) : (
        <>
          {area.width > 0 && (
            <div className="artifact-device" style={{ width: layout.width * layout.scale, height: layout.height * layout.scale }}>
              {frame}
            </div>
          )}
          {readout && <span className="artifact-readout">{readout}</span>}
        </>
      )}
    </div>
  );
}

function ArtifactConsole({ state, path, onClose }: { state: ArtifactState; path: string; onClose: () => void }) {
  const t = useT();
  const writer = state.writer;
  return (
    <div className="artifact-console">
      <div className="artifact-console-head">
        <span className="grow">{t("artifact.console")}</span>
        {writer && state.errorCount > 0 && (
          <button
            className="vlx-btn"
            title={t("artifact.sendToSessionTitle")}
            onClick={() => {
              insertIntoComposer(writer, consoleReport(path, state.console));
              useTermStore.getState().openSession(writer);
            }}
          >
            {t("artifact.sendToSession")}
          </button>
        )}
        <button className="vlx-btn" onClick={state.clearConsole}>
          {t("artifact.clearConsole")}
        </button>
        <button className="artifact-icon-btn" title={t("artifact.closeConsole")} aria-label={t("artifact.closeConsole")} onClick={onClose}>
          <Icons.close size={13} />
        </button>
      </div>
      <div className="artifact-console-list">
        {state.console.length === 0 && <div className="artifact-console-empty">{t("artifact.consoleEmpty")}</div>}
        {state.console.map((e) => (
          <div key={e.id} className={"artifact-console-row " + e.level}>
            <Icons.warn size={13} />
            <span className="artifact-console-text">{e.text}</span>
            <span className="artifact-console-line">{e.line ? `:${e.line}` : ""}</span>
          </div>
        ))}
      </div>
    </div>
  );
}

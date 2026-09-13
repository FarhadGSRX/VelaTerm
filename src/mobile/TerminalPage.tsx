import { ConnectionMenuEntry } from "./ConnectionMenuEntry";
//! Second-level screen: a full-screen session with header, MobileTerminal, and KeyBar.
//!
//! Pin the container to `visualViewport.height`. The iOS keyboard overlays rather than shrinking
//! the layout viewport, so this keeps the terminal and KeyBar visible directly above it. Android's
//! `interactive-widget=resizes-content` already shrinks the viewport to the same value. Height changes
//! reach usePtySession through ResizeObserver: mirror mode only recalculates scaling, while the size
//! owner in fit mode reflows and sends `pty_resize`.

import { useCallback, useEffect, useRef, useState } from "react";
import { StatusIndicator } from "../components/StatusIndicator";
import { ErrorBoundary } from "../components/ErrorBoundary";
import { t, useT } from "../i18n";
import { onPtyExit, onPtyKilled } from "../ipc/events";
import { useTermStore } from "../store/termStore";
import { injectImageFiles } from "../terminal/imageInput";
import { effectiveStatus, supportsChatEngine, type Session } from "../types";
import { MobileSessionBody } from "./MobileSessionBody";

export function TerminalPage({
  session,
  cwd,
  onBack,
}: {
  session: Session;
  cwd?: string;
  onBack: () => void;
}) {
  const tr = useT();
  const status = effectiveStatus(useTermStore((s) => s.runtimes[session.id]));
  const [vvh, setVvh] = useState<number | null>(
    () => window.visualViewport?.height ?? null,
  );

  // Repin the height for keyboard, rotation, and browser-toolbar changes. scrollTo(0, 0) prevents
  // page displacement when iOS focuses the input field.
  useEffect(() => {
    const vv = window.visualViewport;
    if (!vv) return;
    const update = () => {
      setVvh(vv.height);
      window.scrollTo(0, 0);
    };
    vv.addEventListener("resize", update);
    vv.addEventListener("scroll", update);
    return () => {
      vv.removeEventListener("resize", update);
      vv.removeEventListener("scroll", update);
    };
  }, []);

  // Return to the session list when the process exits or another client terminates it. usePtySession
  // also invokes closeSession, but mobile has no tab or split state to close, so navigation is handled here.
  useEffect(() => {
    if (session.engine === "chat" || supportsChatEngine(session.kind)) return;
    let disposed = false;
    const u1 = onPtyExit(session.id, () => {
      if (!disposed) onBack();
    });
    const u2 = onPtyKilled(session.id, () => {
      if (!disposed) onBack();
    });
    return () => {
      disposed = true;
      void u1.then((fn) => fn());
      void u2.then((fn) => fn());
    };
    // MobileApp stabilizes onBack with useCallback; resubscribe only when the session ID changes.
  }, [session.id, session.engine, onBack]);

  // Shared image-injection path for terminal paste/drop and KeyBar: upload through the
  // `save_pasted_image` WS invocation, write the server path to the terminal, and show failures for five seconds.
  const [imgError, setImgError] = useState<string | null>(null);
  const imgErrorTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  useEffect(
    () => () => {
      if (imgErrorTimer.current) clearTimeout(imgErrorTimer.current);
    },
    [],
  );
  const injectImages = useCallback(
    (files: File[]) => {
      void injectImageFiles(session.id, files, session.kind).then((r) => {
        if (!r.fail) return;
        setImgError(t("term.imgUploadFailed", r.fail, r.lastError ?? ""));
        if (imgErrorTimer.current) clearTimeout(imgErrorTimer.current);
        imgErrorTimer.current = setTimeout(() => setImgError(null), 5000);
      });
    },
    [session.id, session.kind],
  );

  return (
    <div className="m-page" style={vvh ? { flex: "none", height: vvh } : undefined}>
      <header className="m-header">
        <button type="button" className="m-back" onClick={onBack}>
          {tr("mobile.back")}
        </button>
        <span className="m-row-dot">
          <StatusIndicator status={status} />
        </span>
        <span className="m-title">{session.name}</span>
        <span className="m-kind">{session.kind}</span>
        {(window as Window & { __VELATERM_CONNECTION_MENU__?: boolean }).__VELATERM_CONNECTION_MENU__ && <details className="m-menu">
          <summary aria-label={tr("mobile.more")}>•••</summary>
          <div className="m-menu-items"><ConnectionMenuEntry /></div>
        </details>}
      </header>
      <div className="m-session-content">
        <ErrorBoundary key={session.id} fallback={(error, retry) => <div className="m-load-error" role="alert">
          <p>{tr("err.renderTitle")}</p><p>{error.message}</p>
          <button type="button" onClick={retry}>{tr("common.retry")}</button>
        </div>}>
          <MobileSessionBody session={session} cwd={cwd} onBack={onBack}
            onImages={injectImages} imgError={imgError} />
        </ErrorBoundary>
      </div>
    </div>
  );
}

import { useEffect, useRef, useState } from "react";
import { Backdrop } from "../components/Backdrop";
import Icons from "../components/Icons";
import { invoke } from "../ipc/transport";
import { env } from "../platform/env";
import { getLocale, useT } from "../i18n";
import { PublicSharingPanel } from "../layout/TitleBar/PublicSharingPanel";
import { sharingText } from "./copy";
import { SharingLink, sharingNavigate, sharingUrl, useSharingLocation } from "./navigation";
import "./shared-projects.css";

const text = (key: string) => sharingText(key, getLocale());

/** Refresh the linked account identity while the panel is open; the server owns the truth. */
function useSharedAccount() {
  const [account, setAccount] = useState<{ id: string; displayName: string } | null>(null);
  const [error, setError] = useState(false);
  const [ready, setReady] = useState(false);
  const [version, setVersion] = useState(0);
  useEffect(() => {
    if (env.isBrowser) return;
    let stopped = false;
    let timer: ReturnType<typeof setTimeout>;
    let generation = 0;
    async function refresh() {
      const current = ++generation;
      clearTimeout(timer);
      try {
        const status = await invoke<{ linked: boolean; account?: { id: string; displayName: string } }>(
          "public_account_status",
        );
        if (stopped || current !== generation) return;
        setAccount(status.account ?? null);
        setError(false);
      } catch {
        if (!stopped && current === generation) {
          setAccount(null);
          setError(true);
        }
      } finally {
        if (!stopped && current === generation) {
          setReady(true);
          timer = setTimeout(() => void refresh(), 15000);
        }
      }
    }
    const changed = () => {
      // Keep the panel mounted during background refreshes so its inputs and login polling survive.
      void refresh();
    };
    window.addEventListener("public-account-changed", changed);
    void refresh();
    return () => {
      stopped = true;
      clearTimeout(timer);
      window.removeEventListener("public-account-changed", changed);
    };
  }, [version]);
  return { account, error, ready, reload: () => setVersion((v) => v + 1) };
}

/** In-app account panel route. The public share URL pages live on the relay and are opened in their own
 * window, so this surface only manages the account link and the ranges this device exposes. */
export function SharedProjectsRoute() {
  const query = new URLSearchParams(useSharingLocation());
  if (env.isBrowser || !query.has("publicAccount")) return null;
  return <SharedProjectsSurface />;
}

function SharedProjectsSurface() {
  const t = useT();
  const { account, error, ready, reload } = useSharedAccount();
  const ref = useRef<HTMLElement>(null);
  const close = () => sharingNavigate(sharingUrl());
  useEffect(() => {
    const previous = document.activeElement as HTMLElement | null;
    ref.current?.focus();
    return () => previous?.focus();
  }, []);
  return (
    <Backdrop onClose={close}>
      <section
        ref={ref}
        tabIndex={-1}
        className="shared-projects-surface"
        role="dialog"
        aria-modal="true"
        aria-labelledby="shared-projects-title"
        onKeyDown={(event) => {
          if (event.key === "Escape") close();
        }}
      >
        <header>
          <h2 id="shared-projects-title">{text("Account")}<span className="shared-projects-badge">{t("common.experimental")}</span></h2>
          <SharingLink className="shared-projects-account" values={{ publicAccount: "1" }}>
            <Icons.account aria-hidden="true" size={14} />
            {account?.displayName ?? text("Account")}
          </SharingLink>
          <SharingLink aria-label={text("projects.close")} className="shared-projects-close">
            <Icons.close aria-hidden="true" size={16} />
          </SharingLink>
        </header>
        <div className="shared-projects-content">
          {!ready ? (
            <p role="status">{text("Loading…")}</p>
          ) : error ? (
            <p role="alert">
              {text("Public sharing service unavailable.")}{" "}
              <button onClick={reload}>{text("projects.retry")}</button>
            </p>
          ) : (
            <PublicSharingPanel />
          )}
        </div>
      </section>
    </Backdrop>
  );
}

//! Artifact cards in a conversation: one per HTML page a tool call wrote, opening it beside the conversation.
//!
//! A conversation view provides an [`ArtifactHost`]; cards rendered elsewhere (or on a phone, where there is
//! no room for a side panel) open the page as a tab instead. Rendering a card also records which session
//! wrote the page, which the preview header and "Send to session" use.

import { createContext, useContext, useEffect } from "react";

import Icons from "../../../../components/Icons";
import { useT } from "../../../../i18n";
import { useTermStore } from "../../../../store/termStore";
import { noteArtifactWriter } from "./artifactWriters";
import "./artifact.css";

export interface ArtifactHost {
  sessionId: string;
  /** Open a page in the panel beside the conversation; null where there is no panel. */
  openBeside: ((path: string) => void) | null;
  /** The page the panel currently shows. */
  openPath: string | null;
}

export const ArtifactHostContext = createContext<ArtifactHost | null>(null);

function baseName(path: string): string {
  return path.split(/[\\/]/).pop() || path;
}

export function ArtifactCards({ paths }: { paths: string[] }) {
  const t = useT();
  const host = useContext(ArtifactHostContext);
  const sessionId = host?.sessionId;
  const key = paths.join("\n");
  useEffect(() => {
    if (!sessionId) return;
    for (const path of key.split("\n")) noteArtifactWriter(path, sessionId);
  }, [sessionId, key]);
  if (paths.length === 0) return null;
  return (
    <div className="artifact-cards">
      {paths.map((path) => (
        <button
          key={path}
          className={"artifact-card" + (host?.openPath === path ? " on" : "")}
          title={host?.openBeside ? t("artifact.cardOpenTitle") : t("artifact.openAsTab")}
          onClick={() => (host?.openBeside ? host.openBeside(path) : useTermStore.getState().openDocTab(path))}
        >
          <span className="artifact-card-icon">
            <Icons.appWindow size={16} />
          </span>
          <span className="artifact-card-main">
            <span className="artifact-card-name">{baseName(path)}</span>
            <span className="artifact-card-kind">{t("artifact.cardKind")}</span>
          </span>
          <span className="artifact-card-open">{t("artifact.cardOpen")}</span>
        </button>
      ))}
    </div>
  );
}

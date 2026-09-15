//! Knowledge base surface: entry reading, provenance, editing and processing history.
import { useEffect, useRef } from "react";
import { useT } from "../../i18n";
import { MemoryLibrary } from "./MemoryLibrary";
import { MemoryCollections } from "./MemoryCollections";
import { MemoryDocument, MemoryEditor, MemorySourceView } from "./MemoryDocument";
import { MemoryCompile, MemoryJobs } from "./MemoryTasks";
import { MemoryLink, useMemoryLocation } from "./navigation";
import { MemoryIcon } from "./shared";
import "./memory.css";
import { KnowledgeVaultDialogs } from "../Notebook/VaultActions";
import { NotebookSurface } from "../Notebook/NotebookSurface";
import { KnowledgeBackLink } from "../Notebook/KnowledgeBack";

export function MemoryRoute() {
  const location = useMemoryLocation();
  const route = new URLSearchParams(location).get("memory");
  const legacy = ["notebooks/open", "notebooks/new"].includes(route ?? "");
  return <><KnowledgeVaultDialogs />{route && !legacy && <MemorySurface route={route} />}</>;
}
function MemorySurface({ route }: { route: string }) {
  const t = useT(); const ref = useRef<HTMLElement>(null);
  const [page, id = "", version] = route.split("/");
  useEffect(() => {
    const previous = document.activeElement as HTMLElement | null;
    ref.current?.focus();
    return () => previous?.focus();
  }, []);
  const compact = page === "compile";
  const library = ["library", "entry", "history"].includes(page);
  const collections = page === "collections" || page === "collection";
  const notebook = page === "notebooks" || page === "notebook";
  return <section ref={ref} data-page={page} tabIndex={-1} className="memory-shell memory-tab-surface" role="tabpanel" aria-label={t(notebook ? "memory.title" : "memory.globalMemory")}>
      {!notebook && <header className="memory-header">
        <div className="memory-brand"><KnowledgeBackLink className="memory-toolbar-button" /><MemoryIcon size={20} /><div><h2 id="memory-heading">{t("memory.globalMemory")}</h2></div></div>
      {!compact && <nav className="memory-nav" aria-label={t("memory.title")}>
        <MemoryLink route="library" className={library ? "active" : ""}>{t("memory.entries")}</MemoryLink>
        <MemoryLink route="jobs" values={{ memoryJobPage: null }} className={["jobs", "job"].includes(page) ? "active" : ""}>{t("memory.jobs")}</MemoryLink>
        <span className="memory-spacer" />
        {!notebook && <MemoryLink route="new" className="btn btn-primary">＋ {t("memory.new")}</MemoryLink>}
      </nav>}
      </header>}
      <div className="memory-body">
        {notebook ? <NotebookSurface route={route} /> : library ? <MemoryLibrary selected={id}>
          {id && <MemoryDocument key={`${page}/${id}/${version ?? ""}`} id={id} version={page === "history" ? Number(version) : undefined} />}
        </MemoryLibrary> : collections ? <div className="memory-workspace"><MemoryCollections key={page} sessionId={page === "collection" ? id : ""} /></div> : <main className="memory-main" key={route}>
          {page === "compile" ? <MemoryCompile sessionId={id} /> : page === "jobs" || page === "job" ? <MemoryJobs id={page === "job" ? id : undefined} /> : page === "source" ? <MemorySourceView id={id} /> : page === "edit" || page === "new" ? <MemoryEditor id={page === "edit" ? id : undefined} /> : <div className="memory-empty">{t("memory.notFound")}</div>}
        </main>}
      </div>
    </section>;
}

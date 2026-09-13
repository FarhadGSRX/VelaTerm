import { useEffect, useRef, useState } from "react";
import { Backdrop } from "../../components/Backdrop";
import { useT, type I18nKey } from "../../i18n";
import { useTermStore } from "../../store/termStore";
import { knowledgeDisable, knowledgeInstall, knowledgeList, knowledgeQuery, knowledgeStart, type GraphSearch, type KnowledgeIndex } from "../../ipc/knowledge";
import { MemoryLink, memoryNavigate } from "../Memory/navigation";
import { KnowledgeLink, knowledgeUrl, useKnowledgeLocation } from "./navigation";
import { knowledgeError, KnowledgeState, useKnowledgeLoad } from "./shared";
import { KnowledgeSymbol } from "./KnowledgeSymbol";
import { KnowledgeExplore, KnowledgeFiles, KnowledgeOverview } from "./KnowledgeWorkspace";
import "../Memory/memory.css";
import "./knowledge.css";

export function KnowledgeRoute() {
  const location = useKnowledgeLocation(); const params = new URLSearchParams(location);
  const projectId = params.get("knowledge");
  return projectId ? <KnowledgeSurface key={projectId} projectId={projectId} params={params} /> : null;
}
const views = ["overview", "symbols", "explore", "files"] as const;
export const directoryName = (root: string) => root.replace(/[/\\]+$/, "").split(/[/\\]/).pop() || root;
function KnowledgeSurface({ projectId, params }: { projectId: string; params: URLSearchParams }) {
  const t = useT(); const ref = useRef<HTMLElement>(null);
  const name = useTermStore(s => s.projects.find(p => p.id === projectId)?.name);
  const { data, error, reload, loading } = useKnowledgeLoad(() => knowledgeList(projectId), [projectId], true);
  const [failure, setFailure] = useState(""); const [busy, setBusy] = useState(false); const [revision, setRevision] = useState(0);
  const indexId = params.get("knowledgeIndex"); const nodeId = params.get("knowledgeNode");
  const index = data?.indexes.find(i => i.id === indexId) ?? (!indexId ? data?.indexes[0] : undefined);
  const requestedView = params.get("knowledgeView") || (nodeId || params.has("knowledgeQuery") ? "symbols" : "overview");
  const view = views.find(v => v === requestedView) || "overview";
  const close = () => memoryNavigate(knowledgeUrl(""));
  useEffect(() => { const previous = document.activeElement as HTMLElement | null; ref.current?.focus(); return () => previous?.focus(); }, []);
  useEffect(() => {
    if (index && !indexId) memoryNavigate(knowledgeUrl(projectId, { knowledgeIndex: index.id }), true);
  }, [index?.id, indexId, projectId]);
  const processing = data?.runtime.status === "installing" || data?.indexes.some(i => ["indexing", "syncing"].includes(i.status));
  useEffect(() => { if (!processing || loading) return; const timer = setTimeout(reload, 1500); return () => clearTimeout(timer); }, [processing, data, loading, reload]);
  const perform = async (fn: () => Promise<unknown>) => {
    setBusy(true); setFailure("");
    try { await fn(); reload(); setRevision(v => v + 1); } catch (e) { setFailure(knowledgeError(e)); } finally { setBusy(false); }
  };
  const indexing = index && ["indexing", "syncing"].includes(index.status);
  return <Backdrop onClose={close}><section ref={ref} className="memory-shell knowledge-shell" data-view={view} tabIndex={-1} role="dialog" aria-modal="true" aria-labelledby="knowledge-heading" onKeyDown={e => {
    if (e.key === "Escape") { e.preventDefault(); e.stopPropagation(); close(); }
    if (e.key === "Tab") {
      const items = [...(ref.current?.querySelectorAll<HTMLElement>('a[href],button:not(:disabled),input:not(:disabled),select:not(:disabled),summary') ?? [])].filter(n => n.getClientRects().length);
      if (items.length && ((!e.shiftKey && document.activeElement === items.at(-1)) || (e.shiftKey && (document.activeElement === items[0] || document.activeElement === ref.current)))) { e.preventDefault(); (e.shiftKey ? items.at(-1) : items[0])?.focus(); }
    }
  }}>
    <header className="knowledge-header"><div><div className="knowledge-eyebrow">{name ?? projectId}<span className="memory-badge">{t("common.experimental")}</span></div><h2 id="knowledge-heading">{t("knowledge.title")}</h2><p>{t("knowledge.intro")}</p></div><div className="knowledge-actions"><MemoryLink route="library" className="btn">{t("memory.title")}</MemoryLink><KnowledgeLink projectId="" className="btn knowledge-close" aria-label={t("common.close")}>×</KnowledgeLink></div></header>
    {!data ? <div className="knowledge-empty" role={error ? "alert" : "status"}>{error || t("common.loading")}{error && <button className="btn" onClick={reload}>{t("common.retry")}</button>}</div> : <div className="knowledge-workspace">
      <div className="knowledge-controls">
        <details className="knowledge-directory" key={index?.id}><summary><span>{t("knowledge.directory")}</span><strong title={index?.root}>{index ? directoryName(index.root) : t("knowledge.directoryMissing")}</strong><span aria-hidden="true">⌄</span></summary><nav className="knowledge-roots" aria-label={t("knowledge.directory")}>{data.indexes.map(item => <KnowledgeLink key={item.id} projectId={projectId} values={{ knowledgeIndex: item.id, knowledgeNode: null, knowledgePage: null, knowledgeLink: null, knowledgeTarget: null, knowledgeLine: null }} title={item.root} aria-label={item.root} aria-current={index?.id === item.id ? "page" : undefined}><strong>{directoryName(item.root)}</strong><small>{item.root}</small><KnowledgeState value={item.status} /></KnowledgeLink>)}</nav></details>
        {index && <div className="knowledge-index-actions"><KnowledgeState value={index.status} /><button className="btn btn-primary" disabled={busy || !data.runtime.available || !!indexing} onClick={() => void perform(() => knowledgeStart(index.id))}>{t(index.enabled ? "knowledge.sync" : "knowledge.enable")}</button>{index.enabled && <button className="btn" disabled={busy} onClick={() => void perform(() => knowledgeDisable(index.id))}>{t("knowledge.disable")}</button>}</div>}
      </div>
      {index && <p className="knowledge-current-path" title={index.root}>{index.root}</p>}
      {failure && <p className="knowledge-warning" role="alert">{failure}</p>}
      {!data.runtime.available ? <div className="knowledge-empty knowledge-card"><span className="knowledge-eyebrow">CodeGraph {data.runtime.version}</span><h3>{t("knowledge.setup")}</h3><p>{t("knowledge.downloadNotice")}</p><button className="btn btn-primary" disabled={busy || !data.runtime.supported || data.runtime.status === "installing"} onClick={() => void perform(knowledgeInstall)}>{t(data.runtime.status === "installing" ? "knowledge.installing" : "knowledge.install")}</button>{data.runtime.error && <p role="alert">{knowledgeError(data.runtime.error)}</p>}</div>
      : !index ? <div className="knowledge-empty knowledge-card">{t("knowledge.directoryMissing")}</div>
      : <>
        <div className="knowledge-metrics">{([['files', index.stats.files], ['symbols', index.stats.nodes], ['edges', index.stats.edges]] as const).map(([key, count]) => <div className="knowledge-metric" key={key}><span>{t(`knowledge.${key}`)}</span><strong>{count?.toLocaleString() ?? "—"}</strong></div>)}</div>
        {index.error && <p className="knowledge-warning" role="alert">{knowledgeError(index.error)}</p>}
        {!index.enabled ? <div className="knowledge-empty knowledge-card"><h3>{t("knowledge.enable")}</h3><p>{t("knowledge.startHelp")}</p><button className="btn btn-primary" disabled={busy} onClick={() => void perform(() => knowledgeStart(index.id))}>{t("knowledge.enable")}</button></div>
        : indexing ? <div className="knowledge-empty knowledge-card" role="status"><span className="knowledge-pulse" aria-hidden="true"/><h3>{t(index.status === "indexing" ? "knowledge.indexing" : "knowledge.syncing")}</h3><p>{t("knowledge.busy")}</p>{index.stats.progress && <div className="knowledge-progress"><progress value={index.stats.progress.current} max={Math.max(1,index.stats.progress.total)} /><span>{index.stats.progress.current.toLocaleString()} / {index.stats.progress.total.toLocaleString()}</span></div>}</div>
        : index.status === "ready" ? <>
          <nav className="knowledge-tabs" aria-label={t("knowledge.title")}>{views.map(v => <KnowledgeLink key={v} projectId={projectId} values={{ knowledgeView: v, knowledgeLink: null }} aria-current={v === view ? "page" : undefined}>{t(`knowledge.${v}` as I18nKey)}</KnowledgeLink>)}</nav>
          <div className={`knowledge-body${nodeId ? " has-node" : ""}`} key={`${index.id}/${revision}`}>
            {view === "overview" && <KnowledgeOverview index={index} />}
            {view === "explore" && <KnowledgeExplore index={index} params={params} />}
            {view === "files" && <KnowledgeFiles index={index} params={params} />}
            {view === "symbols" && <><SymbolSearch index={index} params={params} /><main className="knowledge-main knowledge-card">{nodeId ? <KnowledgeSymbol key={`${index.id}/${nodeId}`} index={index} nodeId={nodeId} linkEntry={params.get("knowledgeLink")} params={params} /> : <div className="knowledge-empty"><span className="knowledge-eyebrow">{t("knowledge.symbols")}</span><h3>{t("knowledge.selectSymbol")}</h3><p>{t("knowledge.analysisNote")}</p></div>}</main></>}
          </div>
        </> : <div className="knowledge-empty knowledge-card"><button className="btn" disabled={busy} onClick={() => void perform(() => knowledgeStart(index.id))}>{t("common.retry")}</button></div>}
      </>}
    </div>}
    <footer className="knowledge-footer"><code>vkb explore "…"</code><span>{t("knowledge.analysisNote")}</span></footer>
  </section></Backdrop>;
}
function SymbolSearch({ index, params }: { index: KnowledgeIndex; params: URLSearchParams }) {
  const t = useT(); const query = params.get("knowledgeQuery") ?? "";
  const page = Math.max(0, Number(params.get("knowledgePage")) || 0); const [draft, setDraft] = useState(query);
  useEffect(() => setDraft(query), [query]);
  const { data, error, reload, loading } = useKnowledgeLoad(() => knowledgeQuery<GraphSearch>(index.id, "search", { query, page }), [index.id, query, page], true);
  return <aside className="knowledge-sidebar knowledge-card"><form onSubmit={e => { e.preventDefault(); memoryNavigate(knowledgeUrl(index.projectId, { knowledgeQuery: draft, knowledgePage: null, knowledgeNode: null, knowledgeLink: null, knowledgeLine: null })); }}><label htmlFor="knowledge-search">{t("knowledge.symbols")}</label><div className="knowledge-search-row"><input id="knowledge-search" className="input" placeholder={t("knowledge.search")} maxLength={500} value={draft} onChange={e => setDraft(e.target.value)} /><button className="btn" type="submit">{t("knowledge.searchButton")}</button></div></form>
    <div className="knowledge-results" aria-busy={loading}>{!data ? <div className="knowledge-empty" role={error ? "alert" : "status"}>{error || t("common.loading")}{error && <button className="btn" onClick={reload}>{t("common.retry")}</button>}</div> : data.nodes.length ? data.nodes.map(node => <KnowledgeLink className="knowledge-node" aria-current={params.get("knowledgeNode") === node.id ? "page" : undefined} key={node.id} projectId={index.projectId} values={{ knowledgeNode: node.id, knowledgeLink: null, knowledgeLine: null, knowledgeTarget: null }}><span className="knowledge-node-kind">{node.kind}</span><strong>{node.name}</strong><small title={node.filePath}>{node.filePath}:{node.startLine}</small></KnowledgeLink>) : <p className="knowledge-empty">{t("knowledge.noResults")}</p>}</div>
    {data && <div className="knowledge-pagination">{page > 0 ? <KnowledgeLink projectId={index.projectId} values={{ knowledgePage: page - 1 }}>{t("common.prev")}</KnowledgeLink> : <span />}<span>{page + 1}</span>{data.hasMore ? <KnowledgeLink projectId={index.projectId} values={{ knowledgePage: page + 1 }}>{t("common.next")}</KnowledgeLink> : <span />}</div>}
  </aside>;
}

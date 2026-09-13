import { useEffect, useState } from "react";
import { useT } from "../../i18n";
import { knowledgeQuery, type GraphExplore, type GraphFiles, type GraphStats, type KnowledgeIndex } from "../../ipc/knowledge";
import { MemoryMarkdown } from "../Memory/shared";
import { memoryNavigate } from "../Memory/navigation";
import { KnowledgeLink, knowledgeUrl } from "./navigation";
import { useKnowledgeLoad } from "./shared";
export function QueryState({ error, reload }: { error: string; reload: () => void }) {
  const t = useT();
  return <div className="knowledge-empty" role={error ? "alert" : "status"}>{error || t("common.loading")}{error && <button className="btn" onClick={reload}>{t("common.retry")}</button>}</div>;
}
export function KnowledgeOverview({ index }: { index: KnowledgeIndex }) {
  const t = useT(); const { data, error, reload, loading } = useKnowledgeLoad(() => knowledgeQuery<GraphStats>(index.id, "status"), [index.id], true);
  useEffect(() => { if (loading || error) return; const timer = setTimeout(reload, 30000); return () => clearTimeout(timer); }, [data, loading, error, reload]);
  if (!data) return <QueryState error={error} reload={reload} />;
  return <div className="knowledge-overview">
    <section className="knowledge-welcome knowledge-card"><div><span className="knowledge-eyebrow">CodeGraph</span><h3>{t("knowledge.explore")}</h3><p>{t("knowledge.selectSymbol")}</p></div><div className="knowledge-actions"><KnowledgeLink className="btn btn-primary" projectId={index.projectId} values={{ knowledgeView: "explore" }}>{t("knowledge.explore")}</KnowledgeLink><KnowledgeLink className="btn" projectId={index.projectId} values={{ knowledgeView: "symbols" }}>{t("knowledge.symbols")}</KnowledgeLink></div></section>
    <div className="knowledge-overview-grid">{([['language', data.filesByLanguage, 'lang'], ['kind', data.nodesByKind, 'kind']] as const).map(([label, counts, field]) => <section className="knowledge-card knowledge-facet" key={label}><h3>{t(`knowledge.${label}`)}</h3>{Object.entries(counts).sort((a,b) => b[1]-a[1]).map(([name, count]) => <KnowledgeLink projectId={index.projectId} values={{ knowledgeView: "symbols", knowledgeQuery: `${field}:${name}`, knowledgePage: null, knowledgeNode: null }} key={name}><span>{name}</span><strong>{count.toLocaleString()}</strong><i style={{ width: `${Math.max(2, count / Math.max(1,...Object.values(counts)) * 100)}%` }} /></KnowledgeLink>)}</section>)}</div>
    <section className="knowledge-card knowledge-live"><div><span className={`knowledge-live-dot ${data.watching ? "active" : ""}`} /><strong>{t(data.watching ? "knowledge.watching" : "knowledge.onDemand")}</strong></div><p>{t("knowledge.liveHelp")}</p><code>vkb explore "…"</code></section>
  </div>;
}
export function KnowledgeExplore({ index, params }: { index: KnowledgeIndex; params: URLSearchParams }) {
  const t = useT(); const query = params.get("knowledgeExplore") || ""; const [draft, setDraft] = useState(query);
  useEffect(() => setDraft(query), [query]);
  return <div className="knowledge-explore"><form className="knowledge-card knowledge-explore-form" onSubmit={e => { e.preventDefault(); if (draft.trim()) memoryNavigate(knowledgeUrl(index.projectId, { knowledgeExplore: draft.trim() })); }}><label htmlFor="knowledge-question">{t("knowledge.explore")}</label><div className="knowledge-search-row"><input id="knowledge-question" className="input" value={draft} onChange={e => setDraft(e.target.value)} maxLength={500} placeholder={t("knowledge.exploreHint")} /><button className="btn btn-primary" disabled={!draft.trim()}>{t("knowledge.explore")}</button></div></form>
    {query ? <ExploreResult key={query} index={index} query={query} /> : <div className="knowledge-empty knowledge-card"><h3>{t("knowledge.exploreHint")}</h3><p>{t("knowledge.analysisNote")}</p><code>vkb explore "…"</code></div>}
  </div>;
}
function ExploreResult({ index, query }: { index: KnowledgeIndex; query: string }) {
  const t = useT(); const { data, error, reload } = useKnowledgeLoad(() => knowledgeQuery<GraphExplore>(index.id, "explore", { query }), [index.id, query], true);
  return <section className="knowledge-card knowledge-explore-result">{data ? <><div className="knowledge-section-heading"><h3>{t("knowledge.results")}</h3><button className="btn" onClick={reload}>{t("knowledge.refresh")}</button></div><p className="knowledge-note">{t("knowledge.analysisNote")}</p><MemoryMarkdown content={data.content.map(c => c.text).join("\n\n")} /></> : <QueryState error={error} reload={reload} />}</section>;
}
export function KnowledgeFiles({ index, params }: { index: KnowledgeIndex; params: URLSearchParams }) {
  const t = useT(); const query = params.get("knowledgeFileQuery") || ""; const page = Math.max(0, Number(params.get("knowledgeFilePage")) || 0);
  const [draft, setDraft] = useState(query); useEffect(() => setDraft(query), [query]);
  const { data, error, reload } = useKnowledgeLoad(() => knowledgeQuery<GraphFiles>(index.id, "files", { query, page }), [index.id, query, page], true);
  return <section className="knowledge-card knowledge-files"><form onSubmit={e => { e.preventDefault(); memoryNavigate(knowledgeUrl(index.projectId, { knowledgeFileQuery: draft, knowledgeFilePage: null })); }}><label htmlFor="knowledge-file-search">{t("knowledge.files")}</label><div className="knowledge-search-row"><input id="knowledge-file-search" className="input" value={draft} onChange={e => setDraft(e.target.value)} placeholder={t("knowledge.search")} maxLength={500} /><button className="btn">{t("knowledge.searchButton")}</button></div></form>
    {!data ? <QueryState error={error} reload={reload} /> : <><div className="knowledge-file-list">{data.files.map(file => <KnowledgeLink projectId={index.projectId} key={file.path} values={{ knowledgeView: "symbols", knowledgeQuery: `path:${file.path}`, knowledgeNode: null, knowledgePage: null, knowledgeLink: null }}><strong>{file.path}</strong><span>{file.language}</span><span>{file.nodeCount} {t("knowledge.symbols")}</span></KnowledgeLink>)}{!data.files.length && <p className="knowledge-empty">{t("knowledge.noResults")}</p>}</div><div className="knowledge-pagination">{page > 0 ? <KnowledgeLink projectId={index.projectId} values={{ knowledgeFilePage: page - 1 }}>{t("common.prev")}</KnowledgeLink> : <span />}<span>{page+1} / {Math.max(1,Math.ceil(data.total/data.pageSize))}</span>{(page+1)*data.pageSize < data.total ? <KnowledgeLink projectId={index.projectId} values={{ knowledgeFilePage: page+1 }}>{t("common.next")}</KnowledgeLink> : <span />}</div></>}
  </section>;
}

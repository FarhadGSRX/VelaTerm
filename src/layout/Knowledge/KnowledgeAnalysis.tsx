import { useEffect, useState } from "react";
import { useT } from "../../i18n";
import Select from "../../components/Select";
import { knowledgeQuery, type GraphCalls, type GraphPath, type GraphSearch, type GraphSlice, type KnowledgeIndex } from "../../ipc/knowledge";
import { memoryNavigate } from "../Memory/navigation";
import { KnowledgeLink, knowledgeUrl } from "./navigation";
import { useKnowledgeLoad } from "./shared";
import { QueryState } from "./KnowledgeWorkspace";

export function KnowledgeCalls({ index, nodeId, direction, params }: { index: KnowledgeIndex; nodeId: string; direction: "callers" | "callees"; params: URLSearchParams }) {
  const t = useT(); const depth = params.get("knowledgeCallDepth");
  const { data, error, reload } = useKnowledgeLoad(() => knowledgeQuery<GraphCalls>(index.id, direction, { nodeId, ...(depth ? { depth: Number(depth) } : {}) }), [index.id, nodeId, direction, depth], true);
  return <section className="knowledge-analysis"><div className="knowledge-section-heading"><h3>{t(`knowledge.${direction}`)}{data && ` · ${data.total.toLocaleString()}`}</h3><label>{t("knowledge.depth")} <Select size="sm" width={72} value={data ? String(data.depth) : depth ?? ""} ariaLabel={t("knowledge.depth")} onChange={value => memoryNavigate(knowledgeUrl(index.projectId, { knowledgeCallDepth: value }))} options={data ? Array.from({ length: data.maxDepth }, (_, i) => ({ value: String(i + 1), label: String(i + 1) })) : [{ value: depth ?? "", label: "—" }]} /></label></div>
    {!data ? <QueryState error={error} reload={reload} /> : <>{data.truncated && <p className="knowledge-warning">{t("knowledge.truncated")}</p>}<div className="knowledge-impact-list">{data.relations.map(({ node, edge }, i) => <KnowledgeLink className="knowledge-edge" projectId={index.projectId} key={`${node.id}/${i}`} values={{ knowledgeNode: node.id, knowledgePanel: "source", knowledgeLine: direction === "callers" ? edge.line ?? null : null, knowledgeLink: null }}><span className="knowledge-node-kind">{node.kind}</span><strong>{node.name}</strong><small>{node.filePath}:{node.startLine}</small>{edge.provenance === "heuristic" && <span className="knowledge-inferred">{t("knowledge.uncertain")}</span>}</KnowledgeLink>)}{!data.relations.length && <p className="knowledge-empty">{t("knowledge.noEdges")}</p>}</div></>}
  </section>;
}

export function KnowledgeImpact({ index, nodeId, params }: { index: KnowledgeIndex; nodeId: string; params: URLSearchParams }) {
  const t = useT(); const depth = params.get("knowledgeDepth");
  const { data, error, reload } = useKnowledgeLoad(() => knowledgeQuery<GraphSlice>(index.id, "impact", { nodeId, ...(depth ? { depth: Number(depth) } : {}) }), [index.id, nodeId, depth], true);
  return <section className="knowledge-analysis"><div className="knowledge-section-heading"><h3>{t("knowledge.impact")}</h3><label>{t("knowledge.depth")} <Select size="sm" width={72} value={data ? String(data.depth) : depth ?? ""} ariaLabel={t("knowledge.depth")} onChange={value => memoryNavigate(knowledgeUrl(index.projectId, { knowledgeDepth: value }))} options={data ? Array.from({length:data.maxDepth},(_,i) => ({ value: String(i + 1), label: String(i + 1) })) : [{ value: depth ?? "", label: "—" }]} /></label></div>
    {!data ? <QueryState error={error} reload={reload} /> : <><div className="knowledge-analysis-summary"><strong>{(data.nodeCount ?? data.nodes.length).toLocaleString()} {t("knowledge.symbols")}</strong><span>{data.fileCount ?? new Set(data.nodes.map(n => n.filePath)).size} {t("knowledge.files")}</span><span>{(data.edgeCount ?? data.edges.length).toLocaleString()} {t("knowledge.edges")}</span></div>{data.truncated && <p className="knowledge-warning">{t("knowledge.truncated")}</p>}<div className="knowledge-impact-list">{data.nodes.map(node => <KnowledgeLink className="knowledge-edge" projectId={index.projectId} key={node.id} values={{ knowledgeNode: node.id, knowledgePanel: "source", knowledgeLine: null, knowledgeTarget: null }}><span className="knowledge-node-kind">{node.kind}</span><strong>{node.name}</strong><small>{node.filePath}:{node.startLine}</small></KnowledgeLink>)}</div></>}
  </section>;
}
export function KnowledgePath({ index, nodeId, params }: { index: KnowledgeIndex; nodeId: string; params: URLSearchParams }) {
  const t = useT(); const target = params.get("knowledgeTarget"); const query = params.get("knowledgeTargetQuery") || "";
  const [draft, setDraft] = useState(query);
  useEffect(() => setDraft(query), [query]);
  return <section className="knowledge-analysis"><form onSubmit={e => { e.preventDefault(); memoryNavigate(knowledgeUrl(index.projectId, { knowledgeTargetQuery: draft, knowledgeTarget: null })); }}><label htmlFor="knowledge-target">{t("knowledge.target")}</label><div className="knowledge-search-row"><input id="knowledge-target" className="input" maxLength={500} placeholder={t("knowledge.target")} value={draft} onChange={e => setDraft(e.target.value)} /><button className="btn">{t("knowledge.searchButton")}</button></div></form>
    {target ? <PathResult key={`${nodeId}/${target}`} index={index} nodeId={nodeId} targetId={target} /> : query ? <PathTargets key={query} index={index} query={query} /> : <p className="knowledge-empty">{t("knowledge.target")}</p>}
  </section>;
}
function PathTargets({ index, query }: { index: KnowledgeIndex; query: string }) {
  const t = useT(); const { data, error, reload } = useKnowledgeLoad(() => knowledgeQuery<GraphSearch>(index.id, "search", { query }), [index.id, query], true);
  if (!data) return <QueryState error={error} reload={reload} />;
  return <div className="knowledge-impact-list">{!data.nodes.length && <p>{t("knowledge.noResults")}</p>}{data.nodes.map(node => <KnowledgeLink className="knowledge-edge" projectId={index.projectId} values={{ knowledgeTarget: node.id }} key={node.id}><strong>{node.name}</strong><small>{node.filePath}:{node.startLine}</small></KnowledgeLink>)}</div>;
}
function PathResult({ index, nodeId, targetId }: { index: KnowledgeIndex; nodeId: string; targetId: string }) {
  const t = useT(); const { data, error, reload } = useKnowledgeLoad(() => knowledgeQuery<GraphPath>(index.id, "path", { nodeId, targetId }), [index.id, nodeId, targetId], true);
  if (!data) return <QueryState error={error} reload={reload} />;
  return data.path ? <ol className="knowledge-path">{data.path.map(({ node, edge }, i) => <li key={node.id}><span className="knowledge-step">{i+1}</span><KnowledgeLink className="knowledge-edge" projectId={index.projectId} values={{ knowledgeNode: node.id, knowledgePanel: "source", knowledgeLine: null, knowledgeTarget: null }}><strong>{node.name}</strong><small>{node.filePath}:{node.startLine}</small>{edge && <small>{edge.kind} · L{edge.line ?? "—"}{edge.provenance ? ` · ${edge.provenance}` : ""}</small>}</KnowledgeLink></li>)}</ol> : <p className="knowledge-empty">{t("knowledge.noPath")}</p>;
}

import { Fragment, useState } from "react";
import { useT } from "../../i18n";
import Select from "../../components/Select";
import { knowledgeLink, knowledgeNode, knowledgeReview, knowledgeUnlink, type CodeEdge, type KnowledgeIndex } from "../../ipc/knowledge";
import { memoryGet, memoryOptions } from "../../ipc/memory";
import { MemoryLink, memoryNavigate } from "../Memory/navigation";
import { MemoryMarkdown } from "../Memory/shared";
import { KnowledgeLink, knowledgeUrl } from "./navigation";
import { KnowledgeSource } from "./KnowledgeSource";
import { KnowledgeCalls, KnowledgeImpact, KnowledgePath } from "./KnowledgeAnalysis";
import { knowledgeError, KnowledgeState, useKnowledgeLoad } from "./shared";

export function KnowledgeSymbol({ index, nodeId, linkEntry, params }: { index: KnowledgeIndex; nodeId: string; linkEntry: string | null; params: URLSearchParams }) {
  const t = useT(); const { data,error,reload } = useKnowledgeLoad(() => knowledgeNode(index.id,nodeId), [index.id,nodeId], true);
  const panel = params.get("knowledgePanel") || "source";
  const selectedLine = Number(params.get("knowledgeLine")) || null;
  const [hoverLine,setHoverLine] = useState<number | null>(null);
  const [failure,setFailure] = useState(""); const [busy,setBusy] = useState(false);
  const perform = async (fn: () => Promise<unknown>) => {
    setBusy(true); setFailure("");
    try { await fn(); memoryNavigate(knowledgeUrl(index.projectId,{ knowledgeLink:null }));reload(); } catch (e) { setFailure(knowledgeError(e)); } finally { setBusy(false); }
  };
  if (!data) return <p className="memory-empty" role={error ? "alert" : "status"}>{error || t("common.loading")}{error && <button className="btn" onClick={reload}>{t("common.retry")}</button>}</p>;
  const existing = data.memories.find((m) => m.entryId === linkEntry);
  return <article className="knowledge-symbol">
    <div className="knowledge-symbol-heading"><KnowledgeLink className="knowledge-mobile-back" projectId={index.projectId} values={{ knowledgeNode:null,knowledgeLink:null }}>← {t("knowledge.symbols")}</KnowledgeLink><div className="knowledge-section-heading"><div><span className="knowledge-node-kind">{data.node.kind} · {data.node.language}</span><h2>{data.node.name}</h2></div><button className="btn" onClick={reload}>{t("knowledge.refresh")}</button></div><p>{data.node.filePath}:{data.node.startLine}–{data.node.endLine}</p><small>{data.node.qualifiedName}</small></div>
    <nav className="knowledge-tabs" aria-label={data.node.name}>{(["source","callers","callees","impact","path"] as const).map(view => <KnowledgeLink key={view} projectId={index.projectId} values={{ knowledgePanel:view, knowledgeLink:null }} aria-current={panel===view ? "page" : undefined}>{t(`knowledge.${view}`)}</KnowledgeLink>)}</nav>
    <p className="knowledge-note">{t("knowledge.analysisNote")}</p>
    {(data.changedDuringRead || data.truncated || data.sourceTruncated) && <p className="knowledge-warning" role="status">{t(data.changedDuringRead ? "knowledge.changed" : "knowledge.truncated")}</p>}
    {panel === "callers" || panel === "callees" ? <KnowledgeCalls index={index} nodeId={nodeId} direction={panel} params={params} /> : panel === "impact" ? <KnowledgeImpact index={index} nodeId={nodeId} params={params} /> : panel === "path" ? <KnowledgePath index={index} nodeId={nodeId} params={params} /> : <div className="knowledge-graph"><EdgeColumn edges={data.incoming} total={data.incomingTotal} index={index} incoming onHover={() => {}} />
      <KnowledgeSource source={data.source} file={data.node.filePath} start={data.node.startLine} projectId={index.projectId} line={hoverLine ?? selectedLine} callLines={data.outgoing.flatMap(e => e.line ? [e.line] : [])} onHover={setHoverLine} />
      <EdgeColumn edges={data.outgoing} total={data.outgoingTotal} index={index} selectedLine={hoverLine ?? selectedLine} onHover={setHoverLine} /></div>}
    <section className="knowledge-memories"><div className="memory-row"><h3>{t("memory.related")}</h3><span className="memory-spacer" /><KnowledgeLink className="btn" projectId={index.projectId} values={{ knowledgeLink:"choose" }}>{t("knowledge.linkMemory")}</KnowledgeLink></div>
      {failure && <p className="knowledge-warning" role="alert">{failure}</p>}
      {!data.memories.length && <p className="knowledge-note">{t("knowledge.noLinks")}</p>}
      {data.memories.map((link) => <div className="knowledge-memory" key={link.id}><MemoryLink route={`entry/${link.entryId}`}>{link.title}</MemoryLink><KnowledgeState value={link.status} /><KnowledgeLink className="btn" projectId={index.projectId} values={{ knowledgeLink:link.entryId }}>{t("knowledge.inspect")}</KnowledgeLink><button className="btn" disabled={busy} onClick={() => void perform(() => knowledgeUnlink(link.entryId,link.id))}>{t("knowledge.unlink")}</button></div>)}
      {linkEntry && <LinkEditor index={index} entryId={linkEntry} busy={busy || data.changedDuringRead} existing={!!existing} onSave={(version) => void perform(() => existing ? knowledgeReview(linkEntry,existing.id,version,data.digest) : knowledgeLink(linkEntry,version,index.id,nodeId,data.digest))} />}
    </section>
  </article>;
}
function EdgeColumn({ edges, total, index, incoming=false, selectedLine, onHover }: { edges: CodeEdge[]; total?: number; index: KnowledgeIndex; incoming?: boolean; selectedLine?: number | null; onHover: (line: number | null) => void }) {
  const t = useT(); const groups = new Map<string, CodeEdge[]>();
  for (const edge of edges) groups.set(edge.kind, [...(groups.get(edge.kind) || []), edge]);
  return <section className={`knowledge-edges ${incoming ? "incoming" : "outgoing"}`}><h3>{t(incoming ? "knowledge.incoming" : "knowledge.outgoing")} <span>{total ?? edges.length}</span></h3>
    {!edges.length && <p className="knowledge-note">{t("knowledge.noEdges")}</p>}
    {[...groups].map(([kind, items]) => <details className="knowledge-relation-group" key={kind} open><summary>{kind}<span>{items.length}</span></summary>{items.map((edge,i) => <Fragment key={`${edge.node.id}/${i}`}>{(i===0 || items[i-1].node.filePath!==edge.node.filePath) && <p className="knowledge-edge-file" title={edge.node.filePath}>{edge.node.filePath.split(/[\\/]/).pop()}</p>}<div className={`knowledge-edge${!incoming && edge.line===selectedLine ? " active" : ""}`} key={`${edge.node.id}/${i}`} onMouseEnter={() => onHover(incoming ? null : edge.line)} onMouseLeave={() => onHover(null)}><KnowledgeLink projectId={index.projectId} values={{ knowledgeNode:edge.node.id,knowledgePanel:"source",knowledgeLine:incoming ? edge.line : null,knowledgeLink:null }}><strong>{edge.node.name}</strong><small title={edge.node.filePath}>{edge.node.filePath}:{edge.node.startLine}</small></KnowledgeLink>{edge.line && <KnowledgeLink className="knowledge-call-line" projectId={index.projectId} values={{ ...(incoming ? { knowledgeNode:edge.node.id } : {}), knowledgeLine:edge.line,knowledgePanel:"source" }}>L{edge.line}</KnowledgeLink>}{edge.inferred && <span className="knowledge-inferred" title={edge.provenance || undefined}>{t("knowledge.uncertain")}</span>}</div></Fragment>)}</details>)}
  </section>;
}
function LinkEditor({ index,entryId,busy,existing,onSave }: { index: KnowledgeIndex; entryId: string; busy: boolean; existing: boolean; onSave: (version: number) => void }) {
  const t = useT(); const { data,error } = useKnowledgeLoad(() => memoryOptions(), []);
  return <section className="knowledge-link-editor" aria-label={t("knowledge.linkMemory")}><div className="memory-row"><h3>{t(existing ? "knowledge.inspect" : "knowledge.linkMemory")}</h3><span className="memory-spacer" /><KnowledgeLink projectId={index.projectId} values={{ knowledgeLink:null }}>{t("common.close")}</KnowledgeLink></div>
    <label>{t("memory.title")}<Select width="100%" value={entryId === "choose" ? "" : entryId} ariaLabel={t("memory.title")} onChange={(value) => memoryNavigate(knowledgeUrl(index.projectId,{ knowledgeLink:value || "choose" }))} options={[{ value: "", label: t("knowledge.chooseMemory") }, ...(data?.catalog.map((entry) => ({ value: entry.id, label: entry.title })) ?? [])]} /></label>
    {error && <p role="alert">{error}</p>}
    {entryId !== "choose" && <MemoryReview key={entryId} id={entryId} busy={busy} existing={existing} onSave={onSave} />}
    {data?.catalog.length === 0 && <MemoryLink route="new">{t("memory.new")}</MemoryLink>}
  </section>;
}
function MemoryReview({ id,busy,existing,onSave }: { id: string; busy: boolean; existing: boolean; onSave: (version: number) => void }) {
  const t = useT(); const { data,error,reload } = useKnowledgeLoad(() => memoryGet(id), [id]);
  if (!data) return <p role={error ? "alert" : "status"}>{error || t("common.loading")}{error && <button className="btn" onClick={reload}>{t("common.retry")}</button>}</p>;
  return <><p>{data.entry.summary}</p><MemoryMarkdown content={data.entry.content} /><p className="knowledge-note">{t("knowledge.reviewHelp")}</p><button className="btn btn-primary" disabled={busy} onClick={() => onSave(data.entry.version)}>{t(existing ? "knowledge.confirmReview" : "knowledge.linkMemory")}</button></>;
}

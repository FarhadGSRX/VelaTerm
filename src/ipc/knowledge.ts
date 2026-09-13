//! CodeGraph protocol. Index state, source and association validation belong to the backend.
import { invoke } from "./transport";
export interface KnowledgeRuntime { version: string; available: boolean; supported: boolean; status: string; error: string }
export interface KnowledgeIndex { id: string; projectId: string; root: string; enabled: boolean; status: string; error: string; stats: { progress?: { current: number; total: number }; files?: number; nodes?: number; edges?: number }; updatedAt: number }
export interface CodeNode { id: string; name: string; qualifiedName: string; kind: string; filePath: string; startLine: number; endLine: number; signature: string | null; language: string }
export interface CodeEdge { inferred?: boolean; node: CodeNode; kind: string; line: number | null; provenance: string | null; metadata: string | null }
export interface CodeLink { id: string; entryId: string; indexId: string; projectId: string; root: string; filePath: string; symbol: string; kind: string; title: string; digest: string; nodeId: string | null; status: string; reviewedAt: number }
export interface CodeDetail { incomingTotal?: number; outgoingTotal?: number; node: CodeNode; incoming: CodeEdge[]; outgoing: CodeEdge[]; source: string; digest: string; changedDuringRead: boolean; truncated: boolean; sourceTruncated: boolean; memories: CodeLink[] }
export const knowledgeList = (projectId: string) => invoke<{ indexes: KnowledgeIndex[]; runtime: KnowledgeRuntime }>("knowledge_list", { projectId });
export const knowledgeInstall = () => invoke<KnowledgeRuntime>("knowledge_install");
export const knowledgeStart = (id: string) => invoke("knowledge_start", { id });
export const knowledgeDisable = (id: string) => invoke("knowledge_disable", { id });
export const knowledgeSearch = (id: string, query: string, page: number) => invoke<{ nodes: CodeNode[]; total: number; pageSize: number }>("knowledge_search", { id, query, page });
export const knowledgeNode = (id: string, nodeId: string) => invoke<CodeDetail>("knowledge_node", { id, nodeId });
export const knowledgeLinks = (entryId: string) => invoke<CodeLink[]>("knowledge_links", { entryId });
export const knowledgeLink = (entryId: string, version: number, indexId: string, nodeId: string, digest: string) => invoke("knowledge_link", { entryId, version, indexId, nodeId, digest });
export const knowledgeUnlink = (entryId: string, id: string) => invoke("knowledge_unlink", { entryId, id });
export const knowledgeReview = (entryId: string, id: string, version: number, digest: string) => invoke("knowledge_review", { entryId, id, version, digest });

export interface GraphStats { nodeCount: number; edgeCount: number; fileCount: number; nodesByKind: Record<string, number>; edgesByKind: Record<string, number>; filesByLanguage: Record<string, number>; dbSizeBytes: number; lastUpdated: number; watching: boolean }
export interface GraphEdge { source: string; target: string; kind: string; line?: number; provenance?: string }
export interface GraphSlice { fileCount: number; nodeCount: number; edgeCount: number; truncated: boolean; depth: number; maxDepth: number; nodes: CodeNode[]; edges: GraphEdge[]; roots: string[] }
export interface GraphPath { path: { node: CodeNode; edge: GraphEdge | null }[] | null }
export interface GraphCalls { relations: { node: CodeNode; edge: GraphEdge }[]; total: number; truncated: boolean; depth: number; maxDepth: number }
export interface GraphFile { path: string; language: string; size: number; nodeCount: number }
export interface GraphSearch { nodes: CodeNode[]; hasMore: boolean; pageSize: number }
export interface GraphFiles { files: GraphFile[]; total: number; pageSize: number }
export interface GraphExplore { content: { type: string; text: string }[] }
export type GraphAction = "status" | "search" | "files" | "explore" | "impact" | "path" | "callers" | "callees";
export const knowledgeQuery = <T>(id: string, action: GraphAction, values: Record<string, string | number> = {}) => invoke<T>("knowledge_query", { id, action, ...values });

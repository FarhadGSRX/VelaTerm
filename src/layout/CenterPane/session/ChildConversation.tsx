import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import { useT } from "../../../i18n";
import { chatChildSnapshot, chatHistorySessionId, chatHistoryChildPath, chatReadCancel, chatToolContextKey, type ChatRow, type ChatSnapshot } from "../../../ipc/chat";

/** Only an expanded parent owns a child page; moving between pages replaces the previous window. */
export function ChildConversation({ row, renderRows }: { row: ChatRow; renderRows: (rows: ChatRow[]) => ReactNode }) {
  const t=useT();
  const [page,setPage]=useState<ChatSnapshot>();
  const [error,setError]=useState<string>();
  const [loading,setLoading]=useState(false);
  const current=useRef<ChatSnapshot | undefined>(undefined);
  const source=useRef(row);source.current=row;
  const section=useRef<HTMLDivElement>(null);
  const reader=useRef<string | undefined>(undefined);
  const request=useRef(0);
  const busy=useRef(false);
  const context=chatToolContextKey(row);
  const load=useCallback(async (direction?:"before"|"after") => {
    if (busy.current) return;
    busy.current=true;setLoading(true);
    const sequence=++request.current;
    const known=current.current;
    const owner=source.current;
    const session=chatHistorySessionId(owner);
    const query=new URLSearchParams(window.location.search);
    const path=[...chatHistoryChildPath(source.current),source.current.id];
    const route=query.getAll("chatChild");
    const anchor=(!query.has("chatSession") || query.get("chatSession")===chatHistorySessionId(source.current)) && path.every((parent,index)=>route[index]===parent)
      ? route[path.length]??query.get("chatRow")??undefined : undefined;
    try {
      const next=await chatChildSnapshot(source.current,{
        ...(direction==="before" ? {before:known?.rows[0]?.id} : direction==="after" ? {after:known?.rows.at(-1)?.id}
          : known?.hasNewer ? {anchor:known.rows.at(-1)?.id} : !known && anchor ? {anchor} : {}),readJobId:reader.current,
      });
      if (sequence!==request.current) {
        if (next.readJobId && session) void chatReadCancel(session,next.readJobId).catch(()=>{});
        return;
      }
      reader.current=next.readJobId;current.current=next;setPage(next);setError(next.historyError??undefined);
    } catch (failure) {if (sequence===request.current) setError(String(failure));}
    finally {if (sequence===request.current) {busy.current=false;setLoading(false);}}
  },[]);
  useEffect(()=>{
    request.current++;busy.current=false;current.current=undefined;setPage(undefined);setError(undefined);
    const session=chatHistorySessionId(source.current);
    void load();
    const timer=setInterval(()=>{
      if (document.visibilityState!=="hidden" && section.current?.getClientRects().length) void load();
    },2000);
    return ()=>{
      clearInterval(timer);request.current++;busy.current=false;
      if (reader.current && session) void chatReadCancel(session,reader.current).catch(()=>{});
      reader.current=undefined;
    };
  },[context,load]);
  return <div className="sv-subagent" ref={section}>
    {page?.hasMore && <button className="vlx-btn" disabled={loading} onClick={()=>void load("before")}>{t("chat.sync.history")}</button>}
    {error && <div role="alert">{error}<button className="vlx-btn" disabled={loading} onClick={()=>void load()}>{t("common.retry")}</button></div>}
    {page?.coverage==="partial" || loading && !page ? <div role="status">{t("chat.history.indexing")}</div> : null}
    {page && renderRows(page.rows)}
    {page?.hasNewer && <button className="vlx-btn" disabled={loading} onClick={()=>void load("after")}>{t("chat.history.newer")}</button>}
  </div>;
}

import { lazy, Suspense } from "react";
import { t } from "../../i18n";
import type { Session } from "../../types";

const Conversation=lazy(()=>import("../CenterPane/session/ChatPane").then(module=>({default:module.ChatPane})));

/** Reuse the shared paged conversation without attaching to or controlling the agent process. */
export function PagedConversation({session,cwd,messageIndex,rowId,mobile=false}:{session:Session;cwd?:string;messageIndex?:number;rowId?:string;mobile?:boolean}) {
  return <Suspense fallback={<div role="status">{t("archive.loadingTranscript")}</div>}>
    <Conversation key={`${session.id}:${rowId ?? messageIndex ?? "recent"}`} session={session} cwd={cwd} mobile={mobile}
      area={{inset:0}} hidden={false} focused multi={false} readOnly historyOnly initialMessageIndex={messageIndex} initialRowId={rowId}
      onActivate={()=>{}} onSplit={()=>{}} onClose={()=>{}} />
  </Suspense>;
}

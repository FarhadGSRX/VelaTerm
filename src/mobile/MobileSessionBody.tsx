import { ChatPane } from "../layout/CenterPane/session/ChatPane";
import { supportsChatEngine, type Session } from "../types";
import { MobileRecordedConversation } from "./MobileRecordedConversation";
import { KeyBar } from "./KeyBar";
import { MobileTerminal } from "./MobileTerminal";

/** App and browser share conversation-first rendering without changing the remote engine. */
export function MobileSessionBody({session,cwd,onBack,onImages,imgError}:{
  session:Session;cwd?:string;onBack:()=>void;
  onImages:(files:File[])=>void;imgError:string|null;
}) {
  if(session.engine === "chat") return <ChatPane key={session.id} session={session} cwd={cwd}
      area={{position:"relative",width:"100%",height:"100%"}}
      hidden={false} focused={true} multi={false} mobile={true}
      onActivate={()=>{}} onSplit={()=>{}} onClose={onBack} />;
  if (supportsChatEngine(session.kind)) return <MobileRecordedConversation key={session.id} session={session} cwd={cwd} />;
  return <>
    <MobileTerminal key={session.id} session={session} cwd={cwd} onImages={onImages} imgError={imgError} />
    <KeyBar sessionId={session.id} onImages={onImages} />
  </>;
}

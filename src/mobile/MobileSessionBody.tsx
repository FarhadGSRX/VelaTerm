import { useCallback, useRef, useState } from "react";
import { flushSync } from "react-dom";
import { ChatPane } from "../layout/CenterPane/session/ChatPane";
import { supportsChatEngine, type Session } from "../types";
import { isShareSurface } from "../ipc/shareBase";
import type { ShareAccess } from "../ipc/chat";
import { MobileRecordedConversation } from "./MobileRecordedConversation";
import { KeyBar } from "./KeyBar";
import { MobileTerminal } from "./MobileTerminal";
import { TerminalComposer } from "./TerminalComposer";
import { focusTerminal, isBracketedPasteMode } from "../terminal/registry";
import type { MobileSessionView } from "./sessionNavigation";

/** Terminal-view agents whose record the phone can read. The terminal shows by default, as on the desktop;
 * the record opens as a separate history page (`sessionView=history`). */
export function hasMobileHistory(session: Session): boolean {
  return session.engine !== "chat" && (session.kind === "kiro" || supportsChatEngine(session.kind));
}

/** Whether a share connection reads the record of a terminal-view agent instead of opening its terminal.
 * Only the host's own account may open terminals through a share; invited accounts read the record until
 * the host switches the session to conversation view. Null while the access is still being asked. */
export function mobileRecordOnly(session: Session, access: ShareAccess | null): boolean | null {
  if (!isShareSurface || session.engine === "chat" || !supportsChatEngine(session.kind)) return false;
  return access ? !access.canUseTerminal : null;
}

/** App and browser show each session the way it runs: conversation view as ChatPane, terminal view as the terminal. */
export function MobileSessionBody({session,cwd,onBack,onImages,imgError,view="terminal",recordOnly=false}:{
  session:Session;cwd?:string;onBack:()=>void;
  onImages:(files:File[])=>void;imgError:string|null;
  view?:MobileSessionView;
  /** From `mobileRecordOnly`; null renders nothing until the share access is known. */
  recordOnly?:boolean|null;
}) {
  if(session.engine === "chat") return <ChatPane key={session.id} session={session} cwd={cwd}
      area={{position:"relative",width:"100%",height:"100%"}}
      hidden={false} focused={true} multi={false} mobile={true}
      onActivate={()=>{}} onSplit={()=>{}} onClose={onBack} />;
  if (recordOnly === null) return null;
  if ((view === "history" && hasMobileHistory(session)) || recordOnly)
    return <MobileRecordedConversation key={session.id} session={session} cwd={cwd} />;
  return <TerminalSession key={session.id} session={session} cwd={cwd} onImages={onImages} imgError={imgError} />;
}

/** Terminal view with the message box. Agent TUIs take whole messages, so a tap on the terminal opens the
 * box; a plain shell keeps typing keystrokes into the terminal, and ✎ opens the box there too. */
function TerminalSession({session,cwd,onImages,imgError}:{
  session:Session;cwd?:string;onImages:(files:File[])=>void;imgError:string|null;
}) {
  const [composing, setComposing] = useState(false);
  const composingRef = useRef(false);
  composingRef.current = composing;
  const inputRef = useRef<HTMLTextAreaElement | null>(null);
  const isShell = session.kind === "terminal";
  // Mount and focus within the same tap: iOS opens the keyboard only for focus during a user gesture.
  const openComposer = useCallback(() => {
    flushSync(() => setComposing(true));
    inputRef.current?.focus();
  }, []);
  const closeComposer = useCallback(() => {
    inputRef.current?.blur();
    setComposing(false);
  }, []);
  const typeDirectly = useCallback(() => {
    flushSync(() => setComposing(false));
    focusTerminal(session.id);
  }, [session.id]);
  const tap = useCallback(() => {
    if (composingRef.current || !isShell) openComposer();
    else focusTerminal(session.id);
  }, [isShell, openComposer, session.id]);
  // Agents are known to read bracketed paste even when the replayed screen has lost the mode switch.
  const bracketed = useCallback(() => !isShell || isBracketedPasteMode(session.id), [isShell, session.id]);
  return <>
    <MobileTerminal session={session} cwd={cwd} onImages={onImages} imgError={imgError} onTap={tap} />
    {composing && <TerminalComposer sessionId={session.id} inputRef={inputRef} bracketed={bracketed}
      onClose={closeComposer} onImages={onImages} />}
    <KeyBar sessionId={session.id} onImages={onImages} agent={!isShell} composing={composing}
      onCompose={composing ? closeComposer : openComposer} onKeyboard={typeDirectly} />
  </>;
}

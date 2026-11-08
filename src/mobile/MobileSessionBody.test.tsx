import {afterEach,describe,it,expect,vi} from "vitest";
import {act,render,screen,cleanup} from "@testing-library/react";
import type {Session} from "../types";
import { supportsChatEngine } from "../types";
vi.mock("../layout/CenterPane/session/ChatPane",()=>({ChatPane:()=> <div data-testid="chat"/>}));
vi.mock("./MobileRecordedConversation",()=>({MobileRecordedConversation:()=> <div data-testid="recorded"/>}));
const props=vi.hoisted(()=>({tap:undefined as undefined|(()=>void),keys:{} as {composing?:boolean;onCompose?:()=>void;onKeyboard?:()=>void},focus:vi.fn()}));
vi.mock("./MobileTerminal",()=>({MobileTerminal:({onTap}:{onTap?:()=>void})=>{props.tap=onTap;return <div data-testid="terminal"/>;}}));
vi.mock("./KeyBar",()=>({KeyBar:(p:typeof props.keys)=>{props.keys=p;return <div data-testid="keys"/>;}}));
vi.mock("./TerminalComposer",()=>({TerminalComposer:({inputRef}:{inputRef:{current:HTMLTextAreaElement|null}})=> <textarea ref={inputRef} aria-label="composer"/>}));
const share=vi.hoisted(()=>({on:false}));
vi.mock("../ipc/shareBase",()=>({get isShareSurface(){return share.on;}}));
vi.mock("../terminal/registry",()=>({focusTerminal:props.focus,isBracketedPasteMode:()=>false}));
import {MobileSessionBody,mobileRecordOnly} from "./MobileSessionBody";
afterEach(()=>{cleanup();props.focus.mockReset();share.on=false;});
describe("手机 App 和浏览器共用的会话内容选择",()=>{
 it("keeps Kiro interactive by default without advertising a live chat engine",()=>{
  expect(supportsChatEngine("kiro")).toBe(false);
  render(<MobileSessionBody session={{id:"kiro",kind:"kiro",engine:"tui"} as Session} onBack={()=>{}} onImages={()=>{}} imgError={null}/>);
  expect(screen.queryByTestId("recorded")).toBeNull();
  expect(screen.queryByTestId("terminal")).not.toBeNull();
  expect(screen.queryByTestId("keys")).not.toBeNull();
  expect(screen.queryByTestId("chat")).toBeNull();
 });
 it("mounts only read-only history when explicitly selected for Kiro",()=>{
  render(<MobileSessionBody session={{id:"kiro",kind:"kiro",engine:"tui"} as Session} view="history" onBack={()=>{}} onImages={()=>{}} imgError={null}/>);
  expect(screen.queryByTestId("recorded")).not.toBeNull();
  expect(screen.queryByTestId("terminal")).toBeNull();
  expect(screen.queryByTestId("keys")).toBeNull();
  expect(screen.queryByTestId("chat")).toBeNull();
 });
 it("已有会话引擎时只挂载会话，不创建 PTY 或快捷键栏",()=>{
  render(<MobileSessionBody session={{id:"chat",kind:"codex",engine:"chat"} as Session} onBack={()=>{}} onImages={()=>{}} imgError={null}/>);
  expect(screen.queryByTestId("chat")).not.toBeNull();expect(screen.queryByTestId("terminal")).toBeNull();expect(screen.queryByTestId("keys")).toBeNull();
 });
 it.each(["shell"])("%s 的终端引擎保持终端，不擅自切换运行方式",kind=>{
  render(<MobileSessionBody session={{id:"term",kind,engine:"tui"} as Session} onBack={()=>{}} onImages={()=>{}} imgError={null}/>);
  expect(screen.queryByTestId("chat")).toBeNull();expect(screen.queryByTestId("terminal")).not.toBeNull();expect(screen.queryByTestId("keys")).not.toBeNull();
 });
 it.each(["claude", "codex", "opencode"])("%s running in terminal view shows its terminal, as on the desktop", kind => {
  render(<MobileSessionBody session={{id:"tui",kind,engine:"tui"} as Session} onBack={()=>{}} onImages={()=>{}} imgError={null}/>);
  expect(screen.queryByTestId("terminal")).not.toBeNull();
  expect(screen.queryByTestId("keys")).not.toBeNull();
  expect(screen.queryByTestId("recorded")).toBeNull();
  expect(screen.queryByTestId("chat")).toBeNull();
 });
 it("opens the transcript of a terminal-view Claude session only from the history link",()=>{
  render(<MobileSessionBody session={{id:"tui",kind:"claude",engine:"tui"} as Session} view="history" onBack={()=>{}} onImages={()=>{}} imgError={null}/>);
  expect(screen.queryByTestId("recorded")).not.toBeNull();
  expect(screen.queryByTestId("terminal")).toBeNull();
 });
 it("keeps an invited share visitor on the transcript and waits for the access check",()=>{
  const view=render(<MobileSessionBody session={{id:"tui",kind:"claude",engine:"tui"} as Session} recordOnly onBack={()=>{}} onImages={()=>{}} imgError={null}/>);
  expect(screen.queryByTestId("recorded")).not.toBeNull();
  expect(screen.queryByTestId("terminal")).toBeNull();
  view.rerender(<MobileSessionBody session={{id:"tui",kind:"claude",engine:"tui"} as Session} recordOnly={null} onBack={()=>{}} onImages={()=>{}} imgError={null}/>);
  expect(screen.queryByTestId("recorded")).toBeNull();
  expect(screen.queryByTestId("terminal")).toBeNull();
 });
 it("opens terminals through a share only for the host's own account",()=>{
  const tui={id:"tui",kind:"claude",engine:"tui"} as Session;
  expect(mobileRecordOnly(tui,null)).toBe(false);
  share.on=true;
  expect(mobileRecordOnly(tui,null)).toBeNull();
  expect(mobileRecordOnly(tui,{canSwitchEngine:true,canUseTerminal:true})).toBe(false);
  expect(mobileRecordOnly(tui,{canSwitchEngine:false,canUseTerminal:false})).toBe(true);
  expect(mobileRecordOnly({...tui,engine:"chat"},{canSwitchEngine:false,canUseTerminal:false})).toBe(false);
  expect(mobileRecordOnly({...tui,kind:"kiro"},{canSwitchEngine:false,canUseTerminal:false})).toBe(false);
 });
 it("opens the message box from a tap on an agent terminal and focuses it",()=>{
  render(<MobileSessionBody session={{id:"agent",kind:"copilot",engine:"tui"} as Session} onBack={()=>{}} onImages={()=>{}} imgError={null}/>);
  expect(screen.queryByLabelText("composer")).toBeNull();
  act(()=>props.tap?.());
  expect(document.activeElement).toBe(screen.getByLabelText("composer"));
  expect(props.focus).not.toHaveBeenCalled();
  act(()=>props.keys.onKeyboard?.());
  expect(screen.queryByLabelText("composer")).toBeNull();
  expect(props.focus).toHaveBeenCalledWith("agent");
 });
 it("keeps keystroke input for a shell tap and opens the box from the key bar",()=>{
  render(<MobileSessionBody session={{id:"sh",kind:"terminal",engine:"tui"} as Session} onBack={()=>{}} onImages={()=>{}} imgError={null}/>);
  act(()=>props.tap?.());
  expect(props.focus).toHaveBeenCalledWith("sh");
  expect(screen.queryByLabelText("composer")).toBeNull();
  act(()=>props.keys.onCompose?.());
  expect(document.activeElement).toBe(screen.getByLabelText("composer"));
  expect(props.keys.composing).toBe(true);
  act(()=>props.keys.onCompose?.());
  expect(screen.queryByLabelText("composer")).toBeNull();
 });
});

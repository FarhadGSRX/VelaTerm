import {afterEach,describe,it,expect,vi} from "vitest";
import {render,screen,cleanup} from "@testing-library/react";
import type {Session} from "../types";
vi.mock("../layout/CenterPane/session/ChatPane",()=>({ChatPane:()=> <div data-testid="chat"/>}));
vi.mock("./MobileRecordedConversation",()=>({MobileRecordedConversation:()=> <div data-testid="recorded"/>}));
vi.mock("./MobileTerminal",()=>({MobileTerminal:()=> <div data-testid="terminal"/>}));
vi.mock("./KeyBar",()=>({KeyBar:()=> <div data-testid="keys"/>}));
import {MobileSessionBody} from "./MobileSessionBody";
afterEach(cleanup);
describe("手机 App 和浏览器共用的会话内容选择",()=>{
 it("已有会话引擎时只挂载会话，不创建 PTY 或快捷键栏",()=>{
  render(<MobileSessionBody session={{id:"chat",kind:"codex",engine:"chat"} as Session} onBack={()=>{}} onImages={()=>{}} imgError={null}/>);
  expect(screen.queryByTestId("chat")).not.toBeNull();expect(screen.queryByTestId("terminal")).toBeNull();expect(screen.queryByTestId("keys")).toBeNull();
 });
 it.each(["shell"])("%s 的终端引擎保持终端，不擅自切换运行方式",kind=>{
  render(<MobileSessionBody session={{id:"term",kind,engine:"tui"} as Session} onBack={()=>{}} onImages={()=>{}} imgError={null}/>);
  expect(screen.queryByTestId("chat")).toBeNull();expect(screen.queryByTestId("terminal")).not.toBeNull();expect(screen.queryByTestId("keys")).not.toBeNull();
 });
 it.each(["claude", "codex", "opencode"])("%s 终端会话读取记录，不挂载终端", kind => {
  render(<MobileSessionBody session={{id:"recorded",kind,engine:"tui"} as Session} onBack={()=>{}} onImages={()=>{}} imgError={null}/>);
  expect(screen.queryByTestId("recorded")).not.toBeNull();
  expect(screen.queryByTestId("terminal")).toBeNull();
  expect(screen.queryByTestId("chat")).toBeNull();
 });
});

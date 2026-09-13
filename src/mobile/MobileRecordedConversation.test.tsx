import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import type { Session } from "../types";
const rpc = vi.hoisted(() => ({readAgentChat:vi.fn(),ptyWrite:vi.fn()}));
vi.mock("../ipc/commands", () => rpc);
vi.mock("../i18n", () => ({useT:()=>(key:string)=>key}));
vi.mock("../layout/sessionViewers/TranscriptViewer", () => ({assistantLabel:()=>"Agent"}));
vi.mock("../layout/CenterPane/session/rows",()=>({MessageBubble:({text}:{text:string})=><p>{text}</p>,ReasoningRow:()=>null,ToolCard:()=>null}));
import {MobileRecordedConversation} from "./MobileRecordedConversation";
const session={id:"remote",kind:"codex",engine:"tui"} as Session;
afterEach(()=>{cleanup();vi.resetAllMocks();vi.useRealTimers()});
describe("手机读取终端会话",()=>{
 it("加载完成后显示记录，发送到原 PTY",async()=>{
  rpc.readAgentChat.mockResolvedValue([{index:0,kind:"assistant",text:"远端继续运行"}]);rpc.ptyWrite.mockResolvedValue(undefined);
  render(<MobileRecordedConversation session={session}/>);
  expect(await screen.findByText("远端继续运行")).toBeTruthy();
  fireEvent.change(screen.getByRole("textbox"),{target:{value:"第一行\n第二行"}});
  fireEvent.click(screen.getByRole("button",{name:"session.send"}));
  await waitFor(()=>expect(rpc.ptyWrite).toHaveBeenCalledTimes(2));
  expect(rpc.ptyWrite.mock.calls).toEqual([["remote","\x1b[200~第一行\n第二行\x1b[201~"],["remote","\r"]]);
  await waitFor(()=>expect((screen.getByRole("textbox") as HTMLTextAreaElement).value).toBe(""));
 });
 it("发送失败保留输入，不自动重发",async()=>{
  rpc.readAgentChat.mockResolvedValue([]);rpc.ptyWrite.mockRejectedValue(new Error("offline"));
  render(<MobileRecordedConversation session={session}/>);await screen.findByText("chat.empty");
  fireEvent.change(screen.getByRole("textbox"),{target:{value:"保留输入"}});
  fireEvent.click(screen.getByRole("button",{name:"session.send"}));
  expect(await screen.findByRole("alert")).toBeTruthy();expect((screen.getByRole("textbox") as HTMLTextAreaElement).value).toBe("保留输入");
  expect(rpc.ptyWrite).toHaveBeenCalledTimes(1);
 });
 it("卸载后不再安排刷新",async()=>{
  vi.useFakeTimers();let resolve!:(value:[])=>void;rpc.readAgentChat.mockReturnValue(new Promise(r=>{resolve=r}));
  const view=render(<MobileRecordedConversation session={session}/>);view.unmount();resolve([]);
  await vi.advanceTimersByTimeAsync(5000);expect(rpc.readAgentChat).toHaveBeenCalledTimes(1);
 });
});

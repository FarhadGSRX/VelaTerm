import {afterEach, describe, expect, it, vi} from "vitest";
import {cleanup, fireEvent, render, screen, waitFor} from "@testing-library/react";
import type {Session} from "../types";
const fixture = vi.hoisted(() => ({read: vi.fn(), write: vi.fn(), crash: false}));
vi.mock("../i18n", () => ({t: (key:string)=>key, useT: ()=>(key:string)=>key}));
vi.mock("../ipc/commands", () => ({readAgentChat: fixture.read, ptyWrite: fixture.write}));
vi.mock("../ipc/events", () => ({onPtyExit: vi.fn(), onPtyKilled: vi.fn()}));
vi.mock("../store/termStore", () => ({useTermStore:(select:(value:unknown)=>unknown)=>select({runtimes:{}})}));
vi.mock("../components/StatusIndicator", () => ({StatusIndicator:()=>null}));
vi.mock("../terminal/imageInput", () => ({injectImageFiles:vi.fn()}));
vi.mock("../layout/sessionViewers/TranscriptViewer", () => ({assistantLabel:()=>"Claude"}));
vi.mock("../layout/CenterPane/session/rows", () => ({MessageBubble:({text}:{text:string})=><p>{text}</p>,ReasoningRow:()=>null,ToolCard:()=>null}));
vi.mock("../layout/CenterPane/session/ChatPane", () => ({ChatPane:()=>{if(fixture.crash)throw new Error("fixture render failure");return <p>recovered</p>}}));
vi.mock("./MobileTerminal", () => ({MobileTerminal:()=>null}));
vi.mock("./KeyBar", () => ({KeyBar:()=>null}));
import {TerminalPage} from "./TerminalPage";
const session = {id:"fixture", name:"Fixture session", kind:"claude", engine:"tui"} as Session;
afterEach(()=>{cleanup();vi.resetAllMocks();fixture.crash=false;delete (window as {__VELATERM_CONNECTION_MENU__?:boolean}).__VELATERM_CONNECTION_MENU__;});
describe("mobile error navigation",()=>{
  it("keeps back and connection management outside a failed conversation",async()=>{
    fixture.read.mockRejectedValue(new Error("Claude transcript file not found"));
    Object.assign(window,{__VELATERM_CONNECTION_MENU__:true});const back=vi.fn();
    render(<TerminalPage session={session} onBack={back}/>);
    await screen.findByRole("alert");
    const header=screen.getByRole("banner");
    expect(header.contains(screen.getByRole("button",{name:"mobile.back"}))).toBe(true);
    expect(screen.getByRole("alert").closest(".m-session-content")).not.toBeNull();
    expect(header.closest(".m-session-content")).toBeNull();
    fireEvent.click(screen.getByLabelText("mobile.more"));
    expect(screen.getByRole("button",{name:"mobile.connections"})).toBeTruthy();
    fireEvent.click(screen.getByRole("button",{name:"mobile.back"}));expect(back).toHaveBeenCalledOnce();
  });
  it("retries recording load and keeps back available while loading",async()=>{
    fixture.read.mockRejectedValueOnce(new Error("missing"));let resolve!:(value:unknown)=>void;
    fixture.read.mockImplementationOnce(()=>new Promise(r=>{resolve=r}));
    const back=vi.fn();render(<TerminalPage session={session} onBack={back}/>);
    await screen.findByRole("alert");fireEvent.click(screen.getByRole("button",{name:"common.retry"}));
    expect(screen.getByRole("status")).toBeTruthy();
    expect(screen.getByRole("button",{name:"mobile.back"})).toBeTruthy();
    resolve([{index:0,kind:"assistant",text:"restored"}]);await screen.findByText("restored");
    expect(screen.queryByRole("alert")).toBeNull();expect(fixture.write).not.toHaveBeenCalled();
  });
  it("contains a render crash below the header and retries only the content",async()=>{
    const error=vi.spyOn(console,"error").mockImplementation(()=>{});
    try {
      fixture.crash=true;const back=vi.fn();render(<TerminalPage session={{...session,engine:"chat"}} onBack={back}/>);
      expect(screen.getByRole("alert")).toBeTruthy();
      fireEvent.click(screen.getByRole("button",{name:"mobile.back"}));expect(back).toHaveBeenCalledOnce();
      fixture.crash=false;fireEvent.click(screen.getByRole("button",{name:"common.retry"}));
      await waitFor(()=>expect(screen.queryByRole("alert")).toBeNull());expect(screen.getByText("recovered")).toBeTruthy();
    } finally {error.mockRestore();}
  });
});

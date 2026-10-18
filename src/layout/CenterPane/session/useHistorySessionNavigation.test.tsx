import { act, cleanup, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";

type State = { treeLoaded: boolean; sessions: { id: string; engine: string }[]; activeSessionId: string | null; dormantSessions: Record<string,true>; openSession: (id: string) => void };
const model = vi.hoisted(() => ({ state: {} as State, listener: undefined as ((state:State,previous:State)=>void) | undefined }));
vi.mock("../../../ipc/shareBase", () => ({ isShareSurface: false }));
vi.mock("../../../store/termStore", () => ({ useTermStore: {
  getState: () => model.state,
  subscribe: (callback:typeof model.listener) => {model.listener=callback;return () => {model.listener=undefined;};},
} }));
import { useHistorySessionNavigation } from "./useHistorySessionNavigation";

function update(patch:Partial<State>) { const previous=model.state;model.state={...previous,...patch};model.listener?.(model.state,previous); }
beforeEach(() => {
  window.history.replaceState(null,"","/?chatSession=chat&session=chat&chatBefore=boundary");
  model.state={treeLoaded:false,sessions:[{id:"chat",engine:"chat"},{id:"terminal",engine:"terminal"}],activeSessionId:null,dormantSessions:{},
    openSession:vi.fn((id:string)=>update({activeSessionId:id,dormantSessions:{}}))};
});
afterEach(() => {cleanup();window.history.replaceState(null,"","/");});

it("selects a copied chat history link after the tree arrives and wakes only its dormant view", () => {
  renderHook(useHistorySessionNavigation);expect(model.state.openSession).not.toHaveBeenCalled();
  act(() => update({treeLoaded:true,dormantSessions:{chat:true}}));
  expect(model.state.openSession).toHaveBeenCalledWith("chat");expect(model.state.activeSessionId).toBe("chat");
  act(() => update({dormantSessions:{chat:true}}));expect(model.state.openSession).toHaveBeenCalledTimes(2);
});

it("does not launch a terminal or resolve a missing session from a history link", () => {
  model.state.treeLoaded=true;window.history.replaceState(null,"","/?chatSession=terminal");renderHook(useHistorySessionNavigation);
  expect(model.state.openSession).not.toHaveBeenCalled();
  window.history.replaceState(null,"","/?chatSession=missing");act(() => window.dispatchEvent(new PopStateEvent("popstate")));
  expect(model.state.openSession).not.toHaveBeenCalled();
});

it("allows another session to be selected and restores the linked conversation on Back", () => {
  model.state.treeLoaded=true;renderHook(useHistorySessionNavigation);const copied=window.location.href;
  act(() => update({activeSessionId:"terminal"}));expect(new URLSearchParams(window.location.search).has("chatSession")).toBe(false);
  act(() => update({sessions:[...model.state.sessions]}));expect(model.state.activeSessionId).toBe("terminal");
  window.history.replaceState(null,"",copied);act(() => window.dispatchEvent(new PopStateEvent("popstate")));
  expect(model.state.activeSessionId).toBe("chat");expect(new URLSearchParams(window.location.search).get("chatBefore")).toBe("boundary");
});

// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { setLang } from "../../i18n";
import { useTermStore } from "../../store/termStore";
import { loadSettings, SETTINGS_KEY } from "../../store/settings";
import { RightPanel } from "../RightPanel/RightPanel";
import { memoryNavigate, memoryUrl } from "../Memory/navigation";

vi.mock("../RightPanel/FilesTab",()=>({FilesTab:()=> <div>File explorer</div>}));
vi.mock("../RightPanel/InfoTab",()=>({InfoTab:()=> <div>Session info</div>}));
vi.mock("../RightPanel/git/GitTab",()=>({GitTab:()=> <div>Git changes</div>}));
vi.mock("../../ipc/notebook",()=>({notebookOverview:async()=>({vaults:[{id:"local",name:"Personal",root:"/notes"}],limits:{},defaultRoot:"/Notes"}),notebookTree:async()=>({vault:{id:"local",name:"Personal",root:"/notes"},nodes:[{kind:"note",path:"Hello.md",absolutePath:"/notes/Hello.md",name:"Hello.md"}],entries:[],trash:[]})}));
const original=useTermStore.getState().setInspectorTab;
beforeEach(()=>{
  setLang("en");window.history.replaceState(null,"","/");
  useTermStore.setState({inspectorTab:"info",setInspectorTab:tab=>useTermStore.setState({inspectorTab:tab})});
});
afterEach(()=>{cleanup();useTermStore.setState({setInspectorTab:original});});

it("keeps Info as the default and exposes four labeled icons",()=>{
  localStorage.removeItem(SETTINGS_KEY);expect(loadSettings().inspectorTab).toBe("info");render(<RightPanel/>);
  for(const name of ["Files","Info","Git","Knowledge Base"]){const link=screen.getByRole("link",{name});expect(link.textContent).toBe("");expect(link.getAttribute("title")).toBe(name);}
  expect(screen.getByRole("link",{name:"Info"}).getAttribute("aria-current")).toBe("page");
});

it("opens knowledge through its right tab URL and keeps the directory in that panel",async()=>{
  render(<RightPanel/>);fireEvent.click(screen.getByRole("link",{name:"Knowledge Base"}));
  await screen.findByRole("link",{name:"Personal"});expect(new URLSearchParams(location.search).get("inspector")).toBe("knowledge");
  expect(new URLSearchParams(location.search).get("memory")).toBe("notebooks");
  expect(screen.getByRole("link",{name:"Session Knowledge Base"})).toBeTruthy();
  expect(screen.getByRole("button",{name:"Open knowledge base"})).toBeTruthy();
  fireEvent.click(screen.getByRole("link",{name:"Personal"}));
  await screen.findByRole("button",{name:"Hello"});expect(document.querySelector(".col-right .nb-file-tree")).not.toBeNull();
  fireEvent.click(screen.getByRole("link",{name:"Info"}));
  await waitFor(()=>expect(useTermStore.getState().inspectorTab).toBe("info"));
  expect(new URLSearchParams(location.search).get("inspector")).toBe("info");
  expect(new URLSearchParams(location.search).get("memory")).toBe("notebook/local");
});

it("lets a tab be reselected after settings sync switched away from the URL's tab",async()=>{
  window.history.replaceState(null,"","/?inspector=info");render(<RightPanel/>);
  act(()=>useTermStore.setState({inspectorTab:"knowledge"}));
  await waitFor(()=>expect(new URLSearchParams(location.search).get("inspector")).toBe("knowledge"));
  fireEvent.click(screen.getByRole("link",{name:"Info"}));
  await waitFor(()=>expect(useTermStore.getState().inspectorTab).toBe("info"));
});

it("keeps an externally selected tab when an unrelated navigation follows",()=>{
  render(<RightPanel/>);
  act(()=>useTermStore.setState({inspectorTab:"knowledge"}));
  expect(new URLSearchParams(location.search).get("inspector")).toBe("knowledge");
  act(()=>memoryNavigate(memoryUrl("notebooks")));
  expect(useTermStore.getState().inspectorTab).toBe("knowledge");
});

it("switches tabs while an unsaved-draft guard blocks page navigation",()=>{
  window.history.replaceState(null,"","/?inspector=info&memory=entry%2Fm1");render(<RightPanel/>);
  const guard=(event:Event)=>event.preventDefault();window.addEventListener("memory:beforeNavigate",guard);
  try {
    fireEvent.click(screen.getByRole("link",{name:"Git"}));
    expect(useTermStore.getState().inspectorTab).toBe("git");
    expect(new URLSearchParams(location.search).get("memory")).toBe("entry/m1");
  } finally { window.removeEventListener("memory:beforeNavigate",guard); }
});

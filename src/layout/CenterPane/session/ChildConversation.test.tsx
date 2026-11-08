import { cleanup,fireEvent,render,screen,waitFor } from "@testing-library/react";
import { afterEach,beforeEach,expect,it,vi } from "vitest";
vi.mock("../../../ipc/chat",async original=>({...await original<typeof import("../../../ipc/chat")>(),chatChildSnapshot:vi.fn(),chatReadCancel:vi.fn(),chatHistorySessionId:()=>"root",chatHistoryChildPath:()=>[]}));
import { chatChildSnapshot,chatReadCancel,type ChatRow,type ChatSnapshot } from "../../../ipc/chat";
import { ChildConversation } from "./ChildConversation";
const row:ChatRow={kind:"tool",id:"parent",name:"Task",input:{},isError:false,status:"completed",childAvailable:true};
const page=(text:string,options?:Partial<ChatSnapshot>)=>({protocolVersion:2,historyGeneration:"g",coverage:"complete",readJobId:"reader",rows:[{kind:"assistant",id:text,text,streaming:false}],queue:[],permissions:[],commands:[],configKeys:[],running:false,...options}) as ChatSnapshot;
beforeEach(()=>{window.history.replaceState(null,"","/");vi.mocked(chatChildSnapshot).mockReset().mockResolvedValue(page("Recent",{hasMore:true}));vi.mocked(chatReadCancel).mockReset().mockResolvedValue();});
afterEach(cleanup);
it("owns one page and cancels only its reader on collapse",async()=>{
  const view=render(<ChildConversation row={row} renderRows={rows=><>{rows.map(row=><span key={row.id}>{"text" in row ? row.text : ""}</span>)}</>} />);
  await screen.findByText("Recent");
  vi.mocked(chatChildSnapshot).mockResolvedValueOnce(page("Older",{hasNewer:true}));
  fireEvent.click(screen.getByRole("button",{name:"Load earlier messages"}));
  await screen.findByText("Older");expect(screen.queryByText("Recent")).toBeNull();
  expect(chatChildSnapshot).toHaveBeenLastCalledWith(row,expect.objectContaining({before:"Recent",readJobId:"reader"}));
  view.unmount();await waitFor(()=>expect(chatReadCancel).toHaveBeenCalledWith("root","reader"));
});
it("opens the nested ancestor named by a copied detail link",async()=>{
  window.history.replaceState(null,"","/?chatSession=root&chatChild=parent&chatChild=nested&chatRow=answer&chatField=field&chatPart=7");
  render(<ChildConversation row={row} renderRows={()=>null} />);
  await waitFor(()=>expect(chatChildSnapshot).toHaveBeenCalledWith(row,expect.objectContaining({anchor:"nested"})));
});
it("does not follow another conversation's copied child path",async()=>{
  window.history.replaceState(null,"","/?chatSession=other&chatChild=parent&chatRow=private");
  render(<ChildConversation row={row} renderRows={()=>null} />);
  await waitFor(()=>expect(chatChildSnapshot).toHaveBeenCalled());
  expect(vi.mocked(chatChildSnapshot).mock.calls[0][1]).not.toHaveProperty("anchor");
});

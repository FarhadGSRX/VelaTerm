import { expect, it, vi, beforeEach } from "vitest";

const { call }=vi.hoisted(()=>({call:vi.fn()}));
vi.mock("./wsClient",()=>({wsClient:{invoke:call,isDiagnosticReady:()=>true},bytesToB64:vi.fn()}));
vi.mock("../i18n",()=>({t:(key:string)=>key}));
import { invoke } from "./transport";

beforeEach(()=>{call.mockReset();});

it("records safe completion metadata while leaving request and response values intact",async()=>{
  const privateValue="synthetic-private-document";
  call.mockImplementation((cmd:string)=>Promise.resolve(cmd==="diagnostic_event" ? true : privateValue));
  expect(await invoke("get_tree",{query:privateValue})).toBe(privateValue);
  const [command,args,trace]=call.mock.calls[0];
  expect(command).toBe("get_tree");
  expect(args.query).toBe(privateValue);
  expect(args).toEqual({query:privateValue});
  expect(trace.requestId).toMatch(/^[0-9a-f-]{36}$/);
  await vi.waitFor(()=>expect(call.mock.calls.length).toBe(2));
  const [event,metadata]=call.mock.calls[1];
  expect(event).toBe("diagnostic_event");
  expect(metadata.requestId).toBe(trace.requestId);
  expect(metadata.status).toBe("success");
  expect(JSON.stringify(metadata)).not.toContain(privateValue);
});

it("diagnostic delivery failure never recursively generates more diagnostic requests",async()=>{
  call.mockImplementation((cmd:string)=>cmd==="diagnostic_event" ? Promise.reject(new Error("synthetic-secret")) : Promise.resolve(7));
  expect(await invoke("get_tree")).toBe(7);
  await new Promise(resolve=>setTimeout(resolve,10));
  expect(call).toHaveBeenCalledTimes(2);
});

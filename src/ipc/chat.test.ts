//! Chat IPC contract tests: snapshots keep image bytes out of the main payload, then resolve stable
//! references through the session-owned attachment endpoint only when a mounted image asks for them.

import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("./transport", () => ({
  invoke: vi.fn(),
  listen: vi.fn(),
}));

import { chatExportContext, chatHistoryGeneration, chatManifest, chatRewind, chatRewindPreview, chatRewindCandidates, chatSnapshot, hasBoundedChatDetails, onChatEvent, readCompleteChatBody,readCompleteQueuedText, resolveChatImage } from "./chat";
import { invoke, listen } from "./transport";

describe("bounded live rows",()=>{
  it("registers live summaries with the manifest contract before consumers receive the event",async()=>{
    const row={kind:"assistant" as const,id:"native-live",text:"Preview",streaming:true,bodyAvailable:true};
    vi.mocked(listen).mockImplementation(async (_name,callback)=>{callback({type:"rows",protocolVersion:2,epoch:7,rows:[row]});return ()=>{};});
    vi.mocked(invoke).mockResolvedValue({row,fields:[],files:[],nextCursor:null} as never);
    await onChatEvent("live-session",event=>{if (event.type==="rows") expect(hasBoundedChatDetails(event.rows[0])).toBe(true);});
    await chatManifest(row);
    expect(invoke).toHaveBeenCalledWith("chat_row",{sessionId:"live-session",epoch:7,protocolVersion:2,historyGeneration:undefined,rowId:"native-live",cursor:undefined});
    vi.mocked(listen).mockReset();vi.mocked(invoke).mockReset();
  });
});

describe("history pages in front of a running timeline",()=>{
  it("reads each row's details and rewinds through the source that row came from",async()=>{
    const history={kind:"tool" as const,id:"call:old",name:"Write",input:{},isError:false,status:"completed" as const,detailAvailable:true,detailProtocol:2 as const,historyGeneration:"generation"};
    const live={kind:"tool" as const,id:"toolu_live",name:"Write",input:{},isError:false,status:"completed" as const,detailAvailable:true,detailProtocol:1 as const};
    vi.mocked(invoke).mockResolvedValue({protocolVersion:2,historyGeneration:"generation",startedAt:7,rows:[history,live],positions:{"call:old":-1,toolu_live:0},queue:[]} as never);
    const [first,second]=(await chatSnapshot("running",{protocolVersion:2})).rows;
    expect(hasBoundedChatDetails(first)).toBe(true);expect(chatHistoryGeneration(first)).toBe("generation");
    expect(hasBoundedChatDetails(second)).toBe(false);expect(chatHistoryGeneration(second)).toBeUndefined();
    vi.mocked(listen).mockImplementation(async (_name,callback)=>{callback({type:"reset",protocolVersion:1,epoch:7,rows:[{...history},{...live}]});return ()=>{};});
    await onChatEvent("running",event=>{
      if (event.type!=="reset" || !event.rows) return;
      expect(hasBoundedChatDetails(event.rows[0])).toBe(true);expect(chatHistoryGeneration(event.rows[0])).toBe("generation");
      expect(hasBoundedChatDetails(event.rows[1])).toBe(false);
    });
    vi.mocked(listen).mockReset();vi.mocked(invoke).mockReset();
  });
});

describe("complete queued drafts",()=>{
  beforeEach(()=>vi.mocked(invoke).mockReset());
  it("reads verified pieces in order without editing the preview",async()=>{
    const item={id:"q",text:"Preview",textDeferred:true,textRevision:"original"};
    vi.mocked(invoke).mockImplementation((_command,args)=>Promise.resolve({text:args?.sequence===0?"Complete ":"original 雪😀",contentRevision:"original",chunkSequence:args?.sequence,eof:args?.sequence===1,sizeBytes:26}));
    expect(await readCompleteQueuedText("session",item)).toBe("Complete original 雪😀");
    expect(invoke).toHaveBeenNthCalledWith(1,"chat_queue_read",{sessionId:"session",id:"q",expectedRevision:"original",sequence:0});
    expect(invoke).toHaveBeenNthCalledWith(2,"chat_queue_read",{sessionId:"session",id:"q",expectedRevision:"original",sequence:1});
    expect(item.text).toBe("Preview");
  });
  it("rejects edited and oversized drafts before requesting more pieces",async()=>{
    const item={id:"q",text:"Preview",textDeferred:true,textRevision:"original"};
    vi.mocked(invoke).mockResolvedValue({text:"Changed",contentRevision:"other",chunkSequence:0,eof:false,sizeBytes:7});
    await expect(readCompleteQueuedText("session",item)).rejects.toThrow("source_changed:");expect(invoke).toHaveBeenCalledTimes(1);
    vi.mocked(invoke).mockReset().mockResolvedValue({text:"Large",contentRevision:"original",chunkSequence:0,eof:false,sizeBytes:9*1024*1024});
    await expect(readCompleteQueuedText("session",item)).rejects.toThrow("detail_requires_download:");expect(invoke).toHaveBeenCalledTimes(1);
  });
  it("keeps short drafts local and stops reading after cancellation",async()=>{
    expect(await readCompleteQueuedText("session",{id:"q",text:"Short"})).toBe("Short");expect(invoke).not.toHaveBeenCalled();
    const controller=new AbortController();vi.mocked(invoke).mockImplementation(()=>{controller.abort();return Promise.resolve({text:"Read",contentRevision:"original",chunkSequence:0,eof:false,sizeBytes:8});});
    await expect(readCompleteQueuedText("session",{id:"q",text:"Preview",textDeferred:true},controller.signal)).rejects.toThrow();
    expect(invoke).toHaveBeenCalledTimes(1);
  });
});

describe("chat snapshot attachments", () => {
  beforeEach(() => vi.mocked(invoke).mockReset());

  it("keeps row and queue references deferred until each image is requested", async () => {
    vi.mocked(invoke).mockImplementation((command, args) => {
      if (command === "chat_snapshot") {
        return Promise.resolve({
          running: true,
          rows: [
            {
              kind: "user",
              id: "u-1",
              text: "look",
              images: [{ mimeType: "image/png", attachmentId: "row:u-1:0" }],
            },
          ],
          queue: [
            {
              id: "q-1",
              text: "next",
              images: [{ mimeType: "image/jpeg", attachmentId: "queue:q-1:0" }],
            },
          ],
          permissions: [],
          commands: [],
          configKeys: [],
        }) as Promise<never>;
      }
      const request = args as { attachmentId?: string } | undefined;
      const jpeg = request?.attachmentId?.startsWith("queue:");
      return Promise.resolve({
        mimeType: jpeg ? "image/jpeg" : "image/png",
        data: jpeg ? "SlBFRw==" : "UE5H",
      }) as Promise<never>;
    });

    const snapshot = await chatSnapshot("session-1");
    expect(snapshot.rows[0]).toMatchObject({
      images: [{ mimeType: "image/png", attachmentId: "row:u-1:0" }],
    });
    expect(snapshot.queue[0]).toMatchObject({
      images: [{ mimeType: "image/jpeg", attachmentId: "queue:q-1:0" }],
    });
    expect(vi.mocked(invoke).mock.calls.map(([command]) => command)).toEqual(["chat_snapshot"]);

    const row = snapshot.rows[0];
    const queued = snapshot.queue[0];
    if (row?.kind !== "user" || !row.images?.[0] || !queued?.images?.[0]) {
      throw new Error("Snapshot fixture did not contain its image references");
    }
    expect(await resolveChatImage(row.images[0])).toEqual({ mimeType: "image/png", data: "UE5H" });
    expect(await resolveChatImage(queued.images[0])).toEqual({ mimeType: "image/jpeg", data: "SlBFRw==" });
    expect(vi.mocked(invoke).mock.calls.map(([command]) => command)).toEqual([
      "chat_snapshot",
      "chat_attachment",
      "chat_attachment",
    ]);
  });

  it("binds historical image reads to the child scope and keeps thumbnails out of a resend",async()=>{
    const image={mimeType:"image/png",attachmentId:"history:original-image",contentRevision:"image-generation:original-image"};
    vi.mocked(invoke).mockImplementation(command=>Promise.resolve(command==="chat_snapshot" ? {
      protocolVersion:2,historyGeneration:"image-branch",running:false,rows:[{kind:"user",id:"image-row",text:"Original input",images:[image]}],queue:[],permissions:[],commands:[],configKeys:[],
    } : {mimeType:"image/png",data:command==="chat_attachment" ? "original-or-preview" : ""}) as never);
    await chatSnapshot("image-session",{protocolVersion:2,childPath:["parent-task"]});
    await resolveChatImage(image);await resolveChatImage(image,true);
    expect(invoke).toHaveBeenCalledWith("chat_attachment",{sessionId:"image-session",attachmentId:"history:original-image",historyGeneration:"image-branch",childPath:["parent-task"],expectedRevision:"image-generation:original-image",purpose:"preview"});
    expect(invoke).toHaveBeenCalledWith("chat_attachment",{sessionId:"image-session",attachmentId:"history:original-image",historyGeneration:"image-branch",childPath:["parent-task"],expectedRevision:"image-generation:original-image",purpose:"original"});
  });

  it("serializes original and preview reads, skips canceled queued work, and caches only thumbnails",async()=>{
    const images=["first","canceled","original"].map(id=>({mimeType:"image/png",attachmentId:`history:image-admission-${id}`}));
    let finishFirst!:(image:{mimeType:string;data:string})=>void;
    let reads=0;
    vi.mocked(invoke).mockImplementation(command=> {
      if (command==="chat_snapshot") return Promise.resolve({protocolVersion:2,historyGeneration:"image-admission-branch",running:false,
        rows:[{kind:"user",id:"image-admission-row",text:"Original input",images}],queue:[],permissions:[],commands:[],configKeys:[]}) as never;
      reads++;
      return (reads===1 ? new Promise(resolve=>{finishFirst=resolve;}) : Promise.resolve({mimeType:"image/png",data:"original-bytes"})) as never;
    });
    await chatSnapshot("image-admission-session",{protocolVersion:2});
    const first=resolveChatImage(images[0]);const controller=new AbortController();
    const canceled=resolveChatImage(images[1],false,controller.signal).catch(error=>error as DOMException);
    const original=resolveChatImage(images[2],true);
    await vi.waitFor(()=>expect(reads).toBe(1));controller.abort();
    finishFirst({mimeType:"image/png",data:"thumbnail-bytes"});
    expect((await first).data).toBe("thumbnail-bytes");expect(await canceled).toMatchObject({name:"AbortError"});
    expect((await original).data).toBe("original-bytes");expect(reads).toBe(2);
    expect((await resolveChatImage(images[0])).data).toBe("thumbnail-bytes");expect(reads).toBe(2);
    await resolveChatImage(images[2],true);expect(reads).toBe(3);
    expect(invoke).not.toHaveBeenCalledWith("chat_attachment",expect.objectContaining({attachmentId:images[1].attachmentId}));
  });
});

describe("streamed context export", () => {
  beforeEach(() => vi.mocked(invoke).mockReset());

  it("returns a download ticket without transferring a document body", async () => {
    vi.mocked(invoke).mockResolvedValue({jobId:"export:job",status:"complete",download:"/api/download?token=fixture",sizeBytes:5_500_000_000} as never);
    const result=await chatExportContext("session","2026-10-09 10:00");
    expect(result).toContain("/api/download?token=fixture");
    expect(invoke).toHaveBeenCalledOnce();
    expect(invoke).toHaveBeenCalledWith("chat_export_start",{sessionId:"session",destPath:undefined,exportedAt:"2026-10-09 10:00"});
  });

  it("cancels a late job when its owner aborts while creation is in flight", async () => {
    let finish!:(value:unknown)=>void;const controller=new AbortController();
    vi.mocked(invoke).mockImplementation(command=>command==="chat_export_start" ? new Promise(resolve=>{finish=resolve;}) as never : Promise.resolve(undefined) as never);
    const pending=chatExportContext("session","2026-10-09",undefined,controller.signal);controller.abort();
    finish({jobId:"export:late",status:"pending"});await expect(pending).rejects.toThrow();
    expect(invoke).toHaveBeenCalledWith("chat_read_cancel",{sessionId:"session",jobId:"export:late"});
  });
});

describe("complete message recall",()=> {
  beforeEach(()=>vi.mocked(invoke).mockReset());
  it("checks the combined original size before requesting any message body",async()=> {
    vi.mocked(invoke).mockImplementation(command=>Promise.resolve(command==="chat_snapshot" ? {protocolVersion:2,historyGeneration:"g",rows:[{kind:"user",id:"long",text:"Preview",bodyAvailable:true}],queue:[],permissions:[],commands:[],configKeys:[]}
      : {protocolVersion:2,fields:[{label:"body",detailRef:"one",contentRevision:"g:one",sizeBytes:8*1024*1024},{label:"body",detailRef:"two",contentRevision:"g:two",sizeBytes:1}],nextCursor:null}) as never);
    const row=(await chatSnapshot("session",{protocolVersion:2})).rows[0];
    await expect(readCompleteChatBody(row)).rejects.toThrow("detail_requires_download:");
    expect(vi.mocked(invoke).mock.calls.map(([command])=>command)).toEqual(["chat_snapshot","chat_row"]);
  });
  it("reads the full UTF-8 original instead of the displayed preview",async()=> {
    const text="雪😀 full original";const sizeBytes=new TextEncoder().encode(text).byteLength;
    vi.mocked(invoke).mockImplementation(command=>Promise.resolve(command==="chat_snapshot" ? {protocolVersion:2,historyGeneration:"g",rows:[{kind:"user",id:"long",text:"Preview",bodyAvailable:true}],queue:[],permissions:[],commands:[],configKeys:[]}
      : command==="chat_row" ? {protocolVersion:2,fields:[{label:"body",detailRef:"body",contentRevision:"g:body",sizeBytes}],nextCursor:null}
      : {contentRevision:"g:body",text,nextCursor:null,eof:true}) as never);
    const row=(await chatSnapshot("session",{protocolVersion:2})).rows[0];expect(await readCompleteChatBody(row)).toBe(text);
  });
  it("restores native multipart separators, attachment placeholders and shell prefixes",async()=> {
    const pieces=[{text:"First original"},{text:"雪😀 second original"},{text:"image/png",literal:"[image]"},{text:"notes.txt",prefix:"[file: ",suffix:"]"},{text:"pwd",prefix:"!"}];
    vi.mocked(invoke).mockImplementation((command,args)=>Promise.resolve(command==="chat_snapshot" ? {protocolVersion:2,historyGeneration:"g",rows:[{kind:"user",id:"long",text:"Preview",bodyAvailable:true}],queue:[],permissions:[],commands:[],configKeys:[]}
      : command==="chat_row" ? {protocolVersion:2,fields:pieces.map((piece,index)=>({label:"body",detailRef:String(index),contentRevision:`g:${index}`,sizeBytes:new TextEncoder().encode(piece.text).byteLength,prefix:piece.prefix,suffix:piece.suffix,literal:piece.literal,append:false})),nextCursor:null}
      : command==="chat_detail_read" ? {contentRevision:`g:${(args as {detailRef:string}).detailRef}`,text:pieces[Number((args as {detailRef:string}).detailRef)].text,nextCursor:null,eof:true} : {}) as never);
    const row=(await chatSnapshot("session",{protocolVersion:2})).rows[0];
    expect(await readCompleteChatBody(row)).toBe("First original\n雪😀 second original\n[image]\n[file: notes.txt]\n!pwd");
    expect(vi.mocked(invoke).mock.calls.filter(([command])=>command==="chat_detail_read")).toHaveLength(5);
  });
  it("joins original append pieces without adding a newline between protocol deltas",async()=> {
    const pieces=["A complete ","雪😀", " original message"];
    vi.mocked(invoke).mockImplementation((command,args)=>Promise.resolve(command==="chat_snapshot" ? {protocolVersion:2,historyGeneration:"g",rows:[{kind:"user",id:"long",text:"Preview",bodyAvailable:true}],queue:[],permissions:[],commands:[],configKeys:[]}
      : command==="chat_row" ? {protocolVersion:2,fields:pieces.map((text,index)=>({label:"body",detailRef:String(index),contentRevision:`g:${index}`,sizeBytes:new TextEncoder().encode(text).byteLength,append:true})),nextCursor:null}
      : command==="chat_detail_read" ? {contentRevision:`g:${(args as {detailRef:string}).detailRef}`,text:pieces[Number((args as {detailRef:string}).detailRef)],nextCursor:null,eof:true} : {}) as never);
    const row=(await chatSnapshot("session",{protocolVersion:2})).rows[0];expect(await readCompleteChatBody(row)).toBe(pieces.join(""));
    expect(vi.mocked(invoke).mock.calls.filter(([command])=>command==="chat_detail_read").map(([,args])=>(args as {detailRef:string}).detailRef)).toEqual(["0","1","2"]);
  });
});

describe("chat rewind transport", () => {
  beforeEach(() => vi.mocked(invoke).mockReset());

  it("keeps the session-owned row id and explicit scope in the shared dispatch contract", async () => {
    vi.mocked(invoke).mockResolvedValue({ prefillText: "try again" } as never);

    await chatRewindPreview("session-1", "u-2");
    await chatRewind("session-1", "u-2", "both");

    expect(vi.mocked(invoke).mock.calls).toEqual([
      ["chat_rewind_preview", { sessionId: "session-1", rowId: "u-2" }],
      ["chat_rewind", { sessionId: "session-1", rowId: "u-2", scope: "both" }],
    ]);
  });

  it("binds indexed confirmations to the selected generation and pages native candidates", async () => {
    vi.mocked(invoke).mockResolvedValue({ historyGeneration: "fixed" } as never);
    await chatRewindPreview("session-1", "native:target", "fixed");
    await chatRewind("session-1", "native:target", "both", "fixed");
    await chatRewindCandidates("session-1", "v2.cursor");
    expect(vi.mocked(invoke).mock.calls).toEqual([
      ["chat_rewind_preview", { sessionId: "session-1", rowId: "native:target", protocolVersion: 2, historyGeneration: "fixed" }],
      ["chat_rewind", { sessionId: "session-1", rowId: "native:target", scope: "both", protocolVersion: 2, historyGeneration: "fixed" }],
      ["chat_rewind_candidates", { sessionId: "session-1", cursor: "v2.cursor" }],
    ]);
  });
});

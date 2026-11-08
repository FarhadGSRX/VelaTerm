import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
vi.mock("../../../ipc/chat", async original => ({ ...await original<typeof import("../../../ipc/chat")>(), chatManifest: vi.fn(), chatDetailRead: vi.fn(), chatDetailLocate: vi.fn(), chatDetailImage: vi.fn(), resolveChatImage: vi.fn(), copyCompleteChatDetail: vi.fn(), downloadCompleteChatDetail: vi.fn() }));
import { copyCompleteChatDetail, downloadCompleteChatDetail, chatDetailImage, resolveChatImage, chatDetailLocate, chatDetailRead, chatManifest, type ChatDetailChunk, type ChatManifest, type ChatRow } from "../../../ipc/chat";
import { DeferredContents, detailTextParts } from "./DeferredContents";

const row: ChatRow = { kind: "tool", id: "change", name: "FileChange", input: {}, isError: false, status: "completed" };
const manifest: ChatManifest = { protocolVersion: 2, historyGeneration: "g", row, nextCursor: null, fields: [], files: [
  { detailRef: "a", contentRevision: "g:a", path: "first.json", kind: "diff" },
  { detailRef: "b", contentRevision: "g:b", path: "second.json", kind: "diff" },
] };
const chunk: ChatDetailChunk = { contentRevision: "g:a", chunkSequence: 0, text: "first part", nextCursor: "v2.1.signed", eof: false, decodedStart: 0, decodedEnd: 10 };
beforeEach(() => {
  window.history.replaceState(null, "", "/");
  vi.mocked(chatManifest).mockReset().mockResolvedValue(manifest);
  vi.mocked(chatDetailRead).mockReset().mockResolvedValue(chunk);
  vi.mocked(copyCompleteChatDetail).mockReset().mockResolvedValue();
  vi.mocked(downloadCompleteChatDetail).mockReset().mockResolvedValue();
  vi.mocked(chatDetailImage).mockReset();vi.mocked(resolveChatImage).mockReset();
  vi.mocked(chatDetailLocate).mockReset().mockImplementation(async (_row,reference,sequence) => ({
    field: { detailRef: reference, contentRevision: `g:${reference}`, label: "input", path: reference, kind: "diff", sizeBytes: 10 }, cursor: `v2.${sequence}.signed`, historyGeneration: "g",
  }));
});
afterEach(cleanup);

it("loads the small manifest without prefetching any file body", async () => {
  render(<DeferredContents row={row} />);
  await screen.findByRole("combobox");
  expect(chatManifest).toHaveBeenCalledWith(row, undefined);
  expect(chatDetailRead).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("combobox"));
  fireEvent.click(screen.getByRole("option", { name: "second.json" }));
  await waitFor(() => expect(chatDetailRead).toHaveBeenCalledOnce());
  expect(chatDetailLocate).toHaveBeenCalledWith(row, "b", 0);
  expect(chatDetailRead).toHaveBeenCalledWith(row, expect.objectContaining({detailRef:"b"}), "v2.0.signed");
});

it("replaces the displayed part and gives it a stable navigation link", async () => {
  window.history.replaceState(null, "", "/?chatRow=change&chatField=a&chatPart=0");
  const { container } = render(<DeferredContents row={row} />);
  await screen.findByText("first part");
  vi.mocked(chatDetailRead).mockResolvedValueOnce({ ...chunk, text: "second part", chunkSequence: 1, nextCursor: null, eof: true });
  const next = screen.getByRole("link", { name: "Next" });
  expect(next.getAttribute("href")).toContain("chatPart=1");
  fireEvent.click(next);
  await screen.findByText("second part");
  expect(screen.queryByText("first part")).toBeNull();
  expect(container.querySelectorAll(".sv-deferred-text")).toHaveLength(1);
});

it("keeps long text nodes within the limit without changing characters or surrogate pairs", () => {
  const text = "x".repeat(4095) + "🚀中文".repeat(10000);
  const parts = detailTextParts(text);
  expect(parts.join("")).toBe(text);
  expect(parts.every(part => part.length <= 4096 && !/[\uD800-\uDBFF]$/.test(part))).toBe(true);
  expect(parts.every(part => new TextEncoder().encode(part).byteLength <= 4096)).toBe(true);
});


it("copies or downloads complete content only after an explicit command", async () => {
  window.history.replaceState(null, "", "/?chatRow=change&chatField=a&chatPart=0");
  render(<DeferredContents row={row} />);
  await screen.findByText("first part");
  expect(copyCompleteChatDetail).not.toHaveBeenCalled();
  expect(downloadCompleteChatDetail).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "Copy complete content" }));
  await waitFor(() => expect(copyCompleteChatDetail).toHaveBeenCalledOnce());
  fireEvent.click(screen.getByRole("button", { name: "Download complete content" }));
  await waitFor(() => expect(downloadCompleteChatDetail).toHaveBeenCalledOnce());
});

it("keeps the original image download available when a bounded thumbnail cannot be decoded",async()=>{
  window.history.replaceState(null,"","/?chatRow=change&chatField=image&chatPart=0");
  const field={detailRef:"image",contentRevision:"g:image",label:"attachment",path:"/payload/content/1/image_url",kind:"image" as const,mimeType:"image/png",sizeBytes:8_000_000};
  vi.mocked(chatManifest).mockResolvedValue({...manifest,files:[],fields:[field]});
  vi.mocked(chatDetailLocate).mockResolvedValue({field,cursor:"first",historyGeneration:"g"});
  vi.mocked(chatDetailImage).mockReturnValue({attachmentId:"history:image",mimeType:"image/png",contentRevision:"g:image"});
  vi.mocked(resolveChatImage).mockRejectedValue(new Error("image_requires_download: image exceeds the preview budget"));
  render(<DeferredContents row={row} />);
  await screen.findByRole("alert");expect(chatDetailRead).not.toHaveBeenCalled();expect(copyCompleteChatDetail).not.toHaveBeenCalled();
  expect(screen.queryByRole("button",{name:"Copy complete content"})).toBeNull();
  fireEvent.click(screen.getByRole("button",{name:"Download complete content"}));
  await waitFor(()=>expect(downloadCompleteChatDetail).toHaveBeenCalledWith(row,field));
});

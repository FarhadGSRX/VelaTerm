// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, renderHook, screen, waitFor, within } from "@testing-library/react";
import { setLang } from "../../i18n";
import { KnowledgeRoute } from "./KnowledgeRoute";
import { useKnowledgeLoad } from "./shared";
import { knowledgeUrl } from "./navigation";

vi.mock("../../ipc/transport", () => ({
  invoke: vi.fn().mockResolvedValue({ indexes: [], runtime: { available: true } }),
  listen: vi.fn().mockResolvedValue(() => {}),
}));
vi.mock("../../store/termStore", () => ({
  useTermStore: (select: (state: unknown) => unknown) => select({ projects: [{ id: "project", name: "Project" }] }),
}));

beforeEach(() => {
  setLang("en");
  window.history.replaceState(null, "", "/?knowledge=project&knowledgeIndex=index&knowledgeQuery=test&session=keep#terminal");
});
afterEach(() => { cleanup(); vi.unstubAllGlobals(); });

describe("代码图谱关闭导航", () => {
  it.each(["tauri://localhost", "tauri://localhost/", "http://localhost:23147/"])("为 %s 生成完整关闭地址", (base) => {
    vi.stubGlobal("window", { location: { href: `${base}?knowledge=project&knowledgeIndex=index&knowledgeQuery=test` } });
    expect(knowledgeUrl("")).toBe(base);
    expect(new URL(knowledgeUrl("")).search).toBe("");
  });

  it.each(["关闭链接", "Escape", "遮罩"])("通过%s关闭窗口并保留其他页面状态", async (method) => {
    render(<KnowledgeRoute />);
    const dialog = screen.getByRole("dialog");
    if (method === "关闭链接") fireEvent.click(screen.getByRole("link", { name: "Close" }));
    else if (method === "Escape") fireEvent.keyDown(dialog, { key: "Escape" });
    else {
      fireEvent.mouseDown(dialog.parentElement!);
      fireEvent.click(dialog.parentElement!);
    }
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(location.search).toBe("?session=keep");
    expect(location.hash).toBe("#terminal");
  });

  it("浏览器后退恢复图谱，前进再次关闭", async () => {
    render(<KnowledgeRoute />);
    fireEvent.click(screen.getByRole("link", { name: "Close" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    window.history.back();
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeNull());
    expect(new URLSearchParams(location.search).get("knowledge")).toBe("project");
    window.history.forward();
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(location.search).toBe("?session=keep");
  });
});

describe("代码图谱后台刷新", () => {
  it("轮询等待期间保留目录和工具栏，完成后更新状态", async () => {
    const { invoke } = await import("../../ipc/transport");
    const data = (status: string) => ({
      runtime: { available: true, status: "ready" },
      indexes: [{ id: "index", root: "/project", enabled: true, status, stats: {} }],
    });
    let finish!: (value: unknown) => void;
    vi.mocked(invoke).mockResolvedValueOnce(data("indexing"))
      .mockImplementationOnce(() => new Promise((resolve) => { finish = resolve; }));
    render(<KnowledgeRoute />);
    const directory = await screen.findByRole("link", { name: "/project" });
    await waitFor(() => expect(finish).toBeDefined(), { timeout: 3000 });
    expect(screen.getByRole("link", { name: "/project" })).toBe(directory);
    expect(screen.queryByText("Loading…")).toBeNull();
    finish(data("disabled"));
    await within(directory).findByText("Disabled");
  });
});


describe("保留刷新数据的加载器", () => {
  it("刷新失败显示错误，重试后恢复", async () => {
    const loader = vi.fn().mockResolvedValueOnce("initial").mockRejectedValueOnce(new Error("offline")).mockResolvedValueOnce("recovered");
    const { result } = renderHook(() => useKnowledgeLoad(loader, ["project"], true));
    await waitFor(() => expect(result.current.data).toBe("initial"));
    act(() => result.current.reload());
    await waitFor(() => expect(result.current.error).not.toBe(""));
    expect(result.current.data).toBeNull();
    act(() => result.current.reload());
    await waitFor(() => expect(result.current.data).toBe("recovered"));
    expect(result.current.error).toBe("");
  });

  it("切换项目清空旧数据，并忽略旧项目的延迟响应", async () => {
    let finish!: (value: string) => void;
    const loader = vi.fn().mockResolvedValueOnce("first")
      .mockImplementationOnce(() => new Promise<string>((resolve) => { finish = resolve; }))
      .mockImplementationOnce(() => new Promise<string>(() => {}));
    const { result, rerender } = renderHook(({ id }) => useKnowledgeLoad(loader, [id], true), { initialProps: { id: "first" } });
    await waitFor(() => expect(result.current.data).toBe("first"));
    act(() => result.current.reload());
    expect(result.current.data).toBe("first");
    rerender({ id: "second" });
    expect(result.current.data).toBeNull();
    await act(async () => finish("late first"));
    expect(result.current.data).toBeNull();
  });
});

describe("代码图谱完整工作区", () => {
  const node = { id: "run", name: "run", qualifiedName: "main.run", kind: "function", language: "typescript", filePath: "src/main.ts", startLine: 1, endLine: 2 };
  beforeEach(async () => {
    const { invoke } = await import("../../ipc/transport");
    vi.mocked(invoke).mockImplementation(async (cmd, args: any) => {
      if (cmd === "knowledge_list") return { runtime: { available: true }, indexes: [{ id: "index", projectId: "project", root: "/project", enabled: true, status: "ready", stats: { nodes: 2, files: 1, edges: 1 } }] };
      if (cmd === "knowledge_node") return { node, source: "export function run() {\n  return helper(); }", incoming: [], outgoing: [{ node: { ...node, id: "helper", name: "helper" }, kind: "calls", line: 2, inferred: true }], memories: [] };
      if (args?.action === "search") return { nodes: [node], pageSize: 40, hasMore: true };
      if (args?.action === "explore") return { content: [{ type: "text", text: "## Flow\nrun calls helper" }] };
      if (args?.action === "impact") return { nodes: [node], edges: [], depth: args.depth || 3, maxDepth: 6 };
      if (args?.action === "callers" || args?.action === "callees") return { relations: [{ node, edge: { kind: "calls", line: 1, provenance: "heuristic" } }], total: 1, depth: args.depth || 3, maxDepth: 6, truncated: false };
      if (args?.action === "path") return { path: null };
      if (args?.action === "files") return { files: [{ path: "src/main.ts", language: "typescript", nodeCount: 2 }], pageSize: 40, total: 1 };
      return { nodesByKind: { function: 2 }, filesByLanguage: { typescript: 1 }, watching: true };
    });
  });
  it("从深层符号链接阅读源码、查看推断标记并切换影响深度", async () => {
    window.history.replaceState(null,"","/?knowledge=project&knowledgeIndex=index&knowledgeView=symbols&knowledgeNode=run");
    render(<KnowledgeRoute />);
    await screen.findByText("Inferred relationship");
    expect(screen.getByRole("link", { name: "L1" }).getAttribute("href")).toContain("knowledgeLine=1");
    fireEvent.click(screen.getByRole("link", { name: "Impact analysis" }));
    await screen.findByText("1 Symbols");
    fireEvent.click(screen.getByRole("combobox", { name: "Traversal depth" }));
    fireEvent.click(screen.getByRole("option", { name: "2" }));
    // jsdom re-activates the wrapping label after the row click, unlike browsers; toggle the popup shut.
    if (screen.queryAllByRole("listbox").length) fireEvent.click(screen.getByRole("combobox", { name: "Traversal depth" }));
    await waitFor(() => expect(location.search).toContain("knowledgeDepth=2"));
    const { invoke } = await import("../../ipc/transport");
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("knowledge_query", { id:"index", action:"impact", nodeId:"run", depth:2 }));
  });
  it("调用遍历方向和深度可直接访问并交给后端查询", async () => {
    window.history.replaceState(null,"","/?knowledge=project&knowledgeIndex=index&knowledgeView=symbols&knowledgeNode=run&knowledgePanel=callers&knowledgeCallDepth=2");
    render(<KnowledgeRoute />);
    await screen.findByText("Callers · 1");
    expect(screen.getByRole("combobox", { name: "Traversal depth" }).textContent).toContain("2");
    fireEvent.click(screen.getByRole("link", { name: "Callees" }));
    await screen.findByText("Callees · 1");
    fireEvent.click(screen.getByRole("combobox", { name: "Traversal depth" }));
    fireEvent.click(screen.getByRole("option", { name: "4" }));
    if (screen.queryAllByRole("listbox").length) fireEvent.click(screen.getByRole("combobox", { name: "Traversal depth" }));
    const { invoke } = await import("../../ipc/transport");
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("knowledge_query", { id:"index", action:"callees", nodeId:"run", depth:4 }));
    expect(location.search).toContain("knowledgeCallDepth=4");
    expect(screen.getByText("Inferred relationship")).toBeTruthy();
  });
  it("探索问题保存在 URL 中，并显示上游返回的内容", async () => {
    window.history.replaceState(null,"","/?knowledge=project&knowledgeIndex=index&knowledgeView=explore");
    render(<KnowledgeRoute />);
    const input=await screen.findByLabelText("Explore code");
    fireEvent.change(input,{target:{value:"run helper"}});
    fireEvent.click(screen.getByRole("button",{name:"Explore code"}));
    await screen.findByText("run calls helper");
    expect(new URLSearchParams(location.search).get("knowledgeExplore")).toBe("run helper");
  });
  it("文件列表提供可直接打开的路径筛选链接", async () => {
    window.history.replaceState(null,"","/?knowledge=project&knowledgeIndex=index&knowledgeView=files");
    render(<KnowledgeRoute />);
    const file=await screen.findByRole("link",{name:/src\/main.ts/});
    expect(new URL(file.getAttribute("href")!).searchParams.get("knowledgeQuery")).toBe("path:src/main.ts");
    fireEvent.click(file);
    await waitFor(() => expect(document.querySelector(".knowledge-node strong")?.textContent).toBe("run"));
    expect(location.search).toContain("knowledgeView=symbols");
  });
});

import {act,renderHook,waitFor,cleanup} from "@testing-library/react";
import {afterEach,expect,it,vi} from "vitest";
import {useMobileTree} from "./useMobileTree";
afterEach(cleanup);
it("请求完成前保持加载，成功后才允许显示空结果",async()=>{
 let finish!:()=>void;const load=vi.fn(()=>new Promise<void>(resolve=>{finish=resolve}));
 const {result}=renderHook(()=>useMobileTree(load));
 expect(result.current.loading).toBe(true);expect(result.current.error).toBeNull();
 await act(async()=>finish());expect(result.current.loading).toBe(false);expect(result.current.error).toBeNull();
});
it("失败不会成为空结果，重试成功后清除错误",async()=>{
 const load=vi.fn().mockRejectedValueOnce(new Error("offline")).mockResolvedValue(undefined);
 const {result}=renderHook(()=>useMobileTree(load));
 await waitFor(()=>expect(result.current.loading).toBe(false));expect(result.current.error).toContain("offline");
 await act(async()=>{await result.current.refresh()});expect(result.current.error).toBeNull();expect(load).toHaveBeenCalledTimes(2);
});

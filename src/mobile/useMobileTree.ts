import { useCallback, useEffect, useRef, useState } from "react";

/** 区分未完成的请求、真实空列表和失败；重试只重新读取服务端数据。 */
export function useMobileTree(load: () => Promise<void>) {
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const generation = useRef(0);
  const refresh = useCallback(async () => {
    const request = ++generation.current;
    setLoading(true);
    setError(null);
    try { await load(); }
    catch (error) { if (request === generation.current) setError(String(error)); }
    finally { if (request === generation.current) setLoading(false); }
  }, [load]);
  useEffect(() => { void refresh(); return () => { ++generation.current; }; }, [refresh]);
  return { loading, error, refresh };
}

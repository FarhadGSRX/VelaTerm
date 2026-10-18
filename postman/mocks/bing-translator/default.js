// Models the anonymous page/form contract observed at https://www.bing.com/translator.
const http = require("http");

http.createServer(async (req, res) => {
  const url = new URL(req.url, "http://localhost");
  if (req.method === "GET" && url.pathname === "/translator") {
    const visits = (await pm.state.get("bing.pageVisits") || 0) + 1;
    await pm.state.set("bing.pageVisits", visits);
    const token = visits === 1 ? "expired" : "fresh";
    res.setHeader("Content-Type", "text/html");
    res.end(`IG:"fixture-ig"; <div data-iid="translator.1"></div><script>var params_AbusePreventionHelper = [123,"${token}",3600000];</script>`);
    return;
  }
  if (req.method !== "POST" || url.pathname !== "/ttranslatev3") {
    res.writeHead(501); res.end("Endpoint not modeled"); return;
  }
  const fields = typeof req.body === "string" ? Object.fromEntries(new URLSearchParams(req.body)) : req.body;
  if (!fields || url.searchParams.get("IG") !== "fixture-ig" || url.searchParams.get("IID") !== "translator.1.1"
    || fields.key !== "123" || fields.fromLang !== "auto-detect" || fields.to !== "zh-Hans"
    || !["Hello", "Hello again."].includes(fields.text)) {
    res.writeHead(400); res.end("Unexpected translation request"); return;
  }
  if (fields.token === "expired") { res.writeHead(401); res.end('{"ShowCaptcha":false}'); return; }
  if (fields.token !== "fresh") { res.writeHead(403); res.end("Unknown page token"); return; }
  res.setHeader("Content-Type", "application/json");
  res.end(JSON.stringify([{ detectedLanguage: { language: "en", score: 1 }, translations: [{ text: "你好", to: "zh-Hans" }] }]));
}).listen(process.env.PORT);

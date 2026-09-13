// Electron bootstrap diagnostics use the same safe metadata contract as the Rust service.
const fs = require("node:fs/promises");
const path = require("node:path");
const { randomUUID } = require("node:crypto");
const EVENTS = new Set(["launcher_failed", "command_ready", "sidecar_exit", "sidecar_restart", "sidecar_restart_failed", "browser_close_failed", "command_install_failed", "startup_failed"]);
function createDiagnostics(directory) {
  const file = path.join(directory, `electron-${process.pid}-${randomUUID()}.log`);
  let pending = Promise.resolve();
  let count = 0;
  let bytes = 0;
  let lastFailure = 0;
  const level = String(process.env.VLX_LOG_LEVEL || "INFO").toUpperCase();
  const record = (event, fields = {}) => {
    if (!EVENTS.has(event) || level === "OFF" || count >= 256) return;
    const severity = event.endsWith("failed") ? "ERROR" : "INFO";
    if (["WARN", "ERROR"].includes(level) && severity === "INFO") return;
    const safe = {};
    for (const key of ["exitCode", "retryCount", "durationMs", "bytes"]) {
      if (Number.isSafeInteger(fields[key])) safe[key] = fields[key];
    }
    const now = new Date();
    const pad = (n) => String(n).padStart(2, "0");
    const timestamp = `${now.getFullYear()}-${pad(now.getMonth()+1)}-${pad(now.getDate())} ${pad(now.getHours())}:${pad(now.getMinutes())}:${pad(now.getSeconds())}`;
    const line = `${timestamp} [${severity.padEnd(5)}] [system] event=${event} ${JSON.stringify(safe)}\n`;
    count++;
    pending = pending.then(async () => {
      await fs.mkdir(directory, { recursive: true, mode: 0o700 });
      if (bytes === 0) {
        const names = (await fs.readdir(directory)).filter(name => /^electron-\d+-[a-f0-9-]{36}\.log(?:\.[1-4])?$/.test(name));
        const files = (await Promise.all(names.map(async name => {
          const target=path.join(directory,name);const stat=await fs.lstat(target).catch(()=>null);
          return stat?.isFile() ? {target,size:stat.size,at:stat.mtimeMs} : null;
        }))).filter(Boolean).sort((a,b)=>a.at-b.at);
        let total=files.reduce((sum,f)=>sum+f.size,0);
        for (const entry of files) {
          if (Date.now()-entry.at>7*86400_000 || total>100*1024*1024) {
            await fs.rm(entry.target,{force:true});total-=entry.size;
          }
        }
      }
      if (bytes + Buffer.byteLength(line) > 10 * 1024 * 1024) {
        await fs.rm(`${file}.4`, { force: true });
        for (let n = 3; n >= 1; n--) await fs.rename(`${file}.${n}`, `${file}.${n+1}`).catch(e => { if (e.code !== "ENOENT") throw e; });
        await fs.rename(file, `${file}.1`);
        bytes = 0;
      }
      await fs.appendFile(file, line, { mode: 0o600 });
      bytes += Buffer.byteLength(line);
      process.stderr.write(line);
    }).catch(() => {
      if (Date.now()-lastFailure>60_000) { lastFailure=Date.now(); process.stderr.write(`${timestamp} [WARN ] [system] event=diagnostic_write status=failed\n`); }
    }).finally(() => { count--; });
  };
  record.flush = () => pending;
  return record;
}
module.exports = { createDiagnostics };

// Adapter-owned transport gate; all audit tools and report generation remain upstream.
import { PassThrough } from 'node:stream';
import { createInterface } from 'node:readline';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { pathToFileURL } from 'node:url';
import { dirname } from 'node:path';

const [server, verifier] = process.argv.slice(2, 4);
if (!server || !verifier) throw new Error('security_guard_configuration_invalid');
const input = process.stdin;
const upstreamInput = new PassThrough();
const shutdown = new AbortController();
// The upstream signal handler closes its transport; also release the real input handle.
for (const signal of ['SIGINT', 'SIGTERM']) {
  process.on(signal, () => { shutdown.abort(); input.destroy(); upstreamInput.end(); });
}
Object.defineProperty(process, 'stdin', { configurable: true, value: upstreamInput });
process.argv = [process.argv[0], server, ...process.argv.slice(4)];
await import(pathToFileURL(server).href);

const execute = promisify(execFile);
for await (const line of createInterface({ input, crlfDelay: Infinity })) {
  let message;
  try { message = JSON.parse(line); } catch { /* Upstream handles malformed input. */ }
  if (message?.method === 'tools/call'
      && message.params?.name === 'complete_codex_security_scan'
      && message.id !== undefined) {
    let result;
    const scanId = message.params.arguments?.scanId;
    try {
      if (typeof scanId !== 'string' || !scanId || scanId.length > 200) {
        throw new Error('security_upstream_invalid');
      }
      const { stdout } = await execute(verifier, [
        '--security-verify-draft', dirname(dirname(server)),
        process.env.CODEX_SECURITY_STATE_DIR ?? '', scanId,
      ], { timeout: 60_000, maxBuffer: 1024 * 1024, windowsHide: true, signal: shutdown.signal });
      result = JSON.parse(stdout);
    } catch {
      result = { ok: false, error: 'security_draft_verification_unavailable' };
    }
    if (shutdown.signal.aborted) break;
    if (result.ok !== true) {
      process.stdout.write(`${JSON.stringify({
        jsonrpc: '2.0', id: message.id,
        result: { isError: true, content: [{ type: 'text', text:
          'Source evidence verification failed before sealing. The scan remains unsealed. '
          + 'Read the exact source lines and correct these locations/excerpts using the existing draft tools; '
          + 'preserve findings and coverage, then retry completion. Do not omit findings to bypass validation. '
          + JSON.stringify(result),
        }] },
      })}\n`);
      continue;
    }
  }
  if (!upstreamInput.write(`${line}\n`)) {
    await new Promise(resolve => upstreamInput.once('drain', resolve));
  }
}
upstreamInput.end();

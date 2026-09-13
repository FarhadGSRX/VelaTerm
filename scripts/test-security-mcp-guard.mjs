// Run after `cargo build --bin velaterm` with the configured upstream Python interpreter.
import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, mkdir, writeFile, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { spawn, execFileSync } from 'node:child_process';
import { createInterface } from 'node:readline';
import { once } from 'node:events';

test('upstream MCP keeps a bad draft unsealed and completes the corrected original finding', { timeout: 120_000 }, async () => {
  const directory = await mkdtemp(join(tmpdir(), 'vlx-security-guard-'));
  const plugin = resolve('src-tauri/resources/codex-security');
  const state = join(directory, 'state');
  const source = join(directory, 'source');
  const env = { ...process.env, CODEX_SECURITY_STATE_DIR: state, PYTHONDONTWRITEBYTECODE: '1' };
  const workbench = (...args) => JSON.parse(execFileSync(env.PYTHON || 'python3',
    [join(plugin, 'scripts/workbench_db.py'), ...args], { env, encoding: 'utf8' }));
  let child;
  try {
    await mkdir(source);
    await writeFile(join(source, 'main.js'), 'export function run(input) {\n  return eval(input);\n}\n');
    const { scan } = workbench('start-prompt-only-scan', '--thread-id', 'guard-test',
      '--target-path', source, '--scope', '.', '--mode', 'standard', '--scan-root', join(directory, 'scans'));
    child = spawn('node', [resolve('src-tauri/resources/security-mcp-guard.mjs'),
      join(plugin, 'mcp/server.mjs'), resolve('src-tauri/target/debug/velaterm'), '--stdio'],
    { env, stdio: ['pipe', 'pipe', 'pipe'] });
    const pending = new Map();
    let nextId = 0;
    let stderr = '';
    child.stderr.on('data', data => { stderr += data; });
    const closed = once(child, 'exit');
    createInterface({ input: child.stdout }).on('line', line => {
      const message = JSON.parse(line);
      if (pending.has(message.id)) {
        pending.get(message.id)(message);
        pending.delete(message.id);
      }
    });
    const request = (method, params) => new Promise(resolveResponse => {
      const id = ++nextId;
      pending.set(id, resolveResponse);
      child.stdin.write(`${JSON.stringify({ jsonrpc: '2.0', id, method, params })}\n`);
    });
    const tool = (name, args) => request('tools/call', { name, arguments: args });
    const initialized = await request('initialize', { protocolVersion: '2024-11-05',
      capabilities: {}, clientInfo: { name: 'vlx-guard-test', version: '1.0' } });
    assert.ok(initialized.result.serverInfo);
    child.stdin.write(`${JSON.stringify({ jsonrpc: '2.0', method: 'notifications/initialized' })}\n`);
    const listed = await request('tools/list', {});
    assert.ok(listed.result.tools.some(tool => tool.name === 'record_codex_security_scan_draft'));
    const unknown = await tool('complete_codex_security_scan', { scanId: '00000000-0000-4000-8000-000000000000' });
    assert.equal(unknown.result.isError, true);
    assert.match(unknown.result.content[0].text, /security_draft_verification_unavailable/);
    const identity = { scanId: scan.scanId, ...(scan.handoffClaimToken ? { handoffClaimToken: scan.handoffClaimToken } : {}) };
    const finding = {
      ruleId: 'code-injection.eval', identity: { anchor: 'main.js/run' },
      title: 'Untrusted input reaches eval', summary: 'Fixture input is evaluated without validation.',
      severity: { level: 'high' }, confidence: { level: 'high', rationale: 'Direct fixture source-to-sink flow.' },
      taxonomy: { category: 'Code injection', cwe: ['CWE-94'] },
      locations: [{ path: 'main.js', startLine: 2, endLine: 2 }],
      codeEvidence: [{ id: 'sink', label: 'Evaluation sink', path: 'main.js', startLine: 2, endLine: 2,
        code: '  return eval(wrong);', explanation: 'Input reaches the evaluation sink.' }],
      remediation: 'Replace evaluation with a constrained parser.', provenance: { source: 'local_plugin' },
    };
    const draft = { ...identity, complete: true, findings: [finding], coverage: {
      completeness: 'complete', surfaces: [{ id: 'entry', label: 'Fixture entrypoint', paths: ['main.js'], disposition: 'reported',
        summary: 'Reviewed the complete fixture entrypoint.' }], explicitExclusions: [], deferred: [],
    } };
    const saved = await tool('record_codex_security_scan_draft', draft);
    assert.ok(!saved.error && !saved.result?.isError, JSON.stringify(saved));
    const before = await readFile(join(scan.scanDir, 'findings.json'), 'utf8');
    const rejected = await tool('complete_codex_security_scan', identity);
    assert.equal(rejected.result.isError, true);
    assert.match(rejected.result.content[0].text, /security_evidence_mismatch/);
    assert.match(rejected.result.content[0].text, /\/findings\/0\/codeEvidence\/0/);
    assert.doesNotMatch(rejected.result.content[0].text, /eval\(wrong\)/);
    assert.equal(await readFile(join(scan.scanDir, 'findings.json'), 'utf8'), before);
    assert.equal(workbench('get-scan', '--scan-id', scan.scanId).scan.progress.status, 'running');
    const unsealed = JSON.parse(await readFile(join(scan.scanDir, 'scan-manifest.json'), 'utf8'));
    assert.equal(unsealed.scan.sealedAt, undefined);
    finding.codeEvidence[0].code = '  return eval(input);';
    finding.codeEvidence[0].endLine = 3;
    const wrongRange = await tool('record_codex_security_scan_draft', draft);
    assert.ok(!wrongRange.error && !wrongRange.result?.isError, JSON.stringify(wrongRange));
    const rangeRejected = await tool('complete_codex_security_scan', identity);
    assert.equal(rangeRejected.result.isError, true);
    assert.match(rangeRejected.result.content[0].text, /"jsonPointer":"\/findings\/0\/codeEvidence\/0"/);
    assert.match(rangeRejected.result.content[0].text, /"expectedEndLine":2/);
    finding.codeEvidence[0].endLine = 2;
    const corrected = await tool('record_codex_security_scan_draft', draft);
    assert.ok(!corrected.error && !corrected.result?.isError, JSON.stringify(corrected));
    const completed = await tool('complete_codex_security_scan', identity);
    assert.ok(!completed.error && !completed.result?.isError, JSON.stringify(completed));
    const sealed = JSON.parse(await readFile(join(scan.scanDir, 'scan-manifest.json'), 'utf8'));
    assert.ok(sealed.scan.sealedAt);
    const findings = JSON.parse(await readFile(join(scan.scanDir, 'findings.json'), 'utf8'));
    assert.equal(findings.findings.length, 1);
    assert.equal(findings.findings[0].codeEvidence[0].code, '  return eval(input);');
    assert.match(await readFile(join(scan.scanDir, 'report.md'), 'utf8'), /Untrusted input reaches eval/);
    const repeated = await tool('complete_codex_security_scan', identity);
    assert.ok(!repeated.error && !repeated.result?.isError, JSON.stringify(repeated));
    child.stdin.end();
    const [code] = await closed;
    assert.equal(code, 0, stderr);
  } finally {
    if (child && child.exitCode === null) {
      child.stdin.destroy();
      child.kill('SIGKILL');
    }
    await rm(directory, { recursive: true, force: true });
  }
});

test('upstream shutdown releases the real stdin on termination', { timeout: 15_000 }, async () => {
  const directory = await mkdtemp(join(tmpdir(), 'vlx-security-guard-stop-'));
  const child = spawn('node', [resolve('src-tauri/resources/security-mcp-guard.mjs'),
    resolve('src-tauri/resources/codex-security/mcp/server.mjs'),
    resolve('src-tauri/target/debug/velaterm'), '--stdio'], {
    env: { ...process.env, CODEX_SECURITY_STATE_DIR: directory }, stdio: ['pipe', 'pipe', 'pipe'],
  });
  try {
    child.stderr.resume();
    const closed = once(child, 'exit');
    const response = once(createInterface({ input: child.stdout }), 'line');
    child.stdin.write(`${JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'initialize', params: {
      protocolVersion: '2024-11-05', capabilities: {}, clientInfo: { name: 'vlx-stop-test', version: '1.0' },
    } })}\n`);
    const [line] = await response;
    assert.ok(JSON.parse(line).result.serverInfo);
    child.kill('SIGTERM');
    const [code] = await closed;
    assert.equal(code, 143); // Original upstream close(143) convention for SIGTERM.
  } finally {
    child.stdin.destroy();
    if (child.exitCode === null) child.kill('SIGKILL');
    await rm(directory, { recursive: true, force: true });
  }
});

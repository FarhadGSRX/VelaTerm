import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
const { JSDOM } = createRequire(import.meta.url)('jsdom');

const source = readFileSync(new URL('../plugins/remote/shared/page-readiness.js', import.meta.url), 'utf8');
async function fixture(html: string, android = false) {
  const dom = new JSDOM(html, { runScripts: 'outside-only', pretendToBeVisual: true });
  const w = dom.window;
  const messages: unknown[] = [];
  Object.defineProperty(w.HTMLElement.prototype, 'getBoundingClientRect', { value() { return { width: 360, height: 640 } } });
  if (android) (w as any).VelaPageReady = { postMessage: (raw: string) => messages.push(JSON.parse(raw)) };
  else (w as any).webkit = { messageHandlers: { VelaPageReady: { postMessage: async (body: unknown) => messages.push(body) } } };
  await new Promise<void>(yes => w.addEventListener('load', () => yes(), { once: true }));
  w.eval(source);
  const paint = () => new Promise<void>(yes => w.requestAnimationFrame(() => w.requestAnimationFrame(() => w.requestAnimationFrame(() => yes()))));
  return { w, messages, paint, close: () => { w.dispatchEvent(new w.Event('pagehide')); w.close() } };
}

test('HTML load with an empty root never removes native loading navigation', async () => {
  const f = await fixture('<div id="root"></div>');
  try { await f.paint(); assert.equal(f.messages.length, 0) } finally { f.close() }
});

test('both native platforms wait for the mobile tree and only signal readiness once', async () => {
  for (const android of [false, true]) {
    const f = await fixture('<div class="m-app"><header class="m-header"><button>Menu</button></header><div class="m-load-status">Loading</div><div class="m-list" aria-busy="true"></div></div>', android);
    try {
      await f.paint(); assert.equal(f.messages.length, 0);
      f.w.document.querySelector('.m-load-status')!.remove();
      await f.paint(); assert.equal(f.messages.length, 0);
      f.w.document.querySelector('.m-list')!.setAttribute('aria-busy', 'false');
      await f.paint(); assert.equal(f.messages.length, 1);
      f.w.document.body.append(f.w.document.createElement('div'));
      await f.paint(); assert.equal(f.messages.length, 1);
    } finally { f.close() }
  }
});

test('a usable login page is revealed, but a hidden form is not', async () => {
  const f = await fixture('<div class="login-screen" style="display:none"><input type="password"><button>Back</button></div>');
  try {
    await f.paint(); assert.equal(f.messages.length, 0);
    (f.w.document.querySelector('.login-screen') as HTMLElement).style.display = 'block';
    await f.paint(); assert.equal(f.messages.length, 1);
  } finally { f.close() }
});

test('leaving a blank page cancels its readiness observer', async () => {
  const f = await fixture('<div id="root"></div>');
  try {
    f.w.dispatchEvent(new f.w.Event('pagehide'));
    f.w.document.querySelector('#root')!.innerHTML = '<button>Ready too late</button>';
    await f.paint(); assert.equal(f.messages.length, 0);
  } finally { f.close() }
});

import test from 'node:test';
import assert from 'node:assert/strict';
import { ConnectionAttempt } from './connection-attempt.ts';

test('return invalidates a pending connection without waiting for it, including late failures', async () => {
  const attempts = new ConnectionAttempt();
  let reject!: (error: Error) => void;
  let failures = 0, finishes = 0;
  const old = attempts.run(() => new Promise((_, no) => { reject = no }), () => failures++, () => finishes++);
  attempts.cancel();
  await attempts.run(async () => {}, () => failures++, () => finishes++);
  assert.equal(finishes, 1);
  reject(new Error('Old connection failed'));
  await old;
  assert.equal(failures, 0); assert.equal(finishes, 1);
});

test('a cancelled success cannot dismiss the next connection loading screen', async () => {
  const attempts = new ConnectionAttempt();
  let resolve!: () => void;
  let finishes = 0;
  const old = attempts.run(() => new Promise<void>(yes => { resolve = yes }), value => assert.fail(String(value)), () => finishes++);
  attempts.cancel(); resolve(); await old;
  assert.equal(finishes, 0);
});

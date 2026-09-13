import test from 'node:test';
import assert from 'node:assert/strict';
import { NotificationNavigation, type NotificationTarget } from './notification-navigation.ts';

test('notification taps preserve the connection and session for URL SSH and account destinations', async () => {
  for (const id of ['url-connection', 'ssh-connection', 'account_625f1418-5e21-45fe-a019-094927f3b69e']) {
    const calls: unknown[] = [];
    await new NotificationNavigation().open({id, sessionId:'session-2'}, {
      cancelConnection: () => { calls.push('cancel') },
      disconnect: async () => { calls.push('disconnect') },
      showConnections: async () => { calls.push('list') },
      connect: async target => { calls.push(target) },
    });
    assert.deepEqual(calls, ['cancel', 'disconnect', 'list', {id, sessionId:'session-2'}]);
  }
});

test('a newer tap wins while a previous connection is disconnecting', async () => {
  const navigation = new NotificationNavigation();
  let finish!: () => void;
  const opened: NotificationTarget[] = [];
  const actions = {cancelConnection() {}, disconnect: async () => {}, showConnections: async () => {},
    connect: async (target: NotificationTarget) => { opened.push(target) }};
  const first = navigation.open({id:'old',sessionId:'old-session'}, {...actions, disconnect: () => new Promise<void>(resolve => {finish=resolve})});
  await navigation.open({id:'new',sessionId:'new-session'}, actions);
  finish(); await first;
  assert.deepEqual(opened, [{id:'new',sessionId:'new-session'}]);
});

test('return cancels a notification reconnect before it opens a remote page', async () => {
  const navigation = new NotificationNavigation();
  let finish!: () => void; let opened = false;
  const work = navigation.open({id:'connection',sessionId:'session'}, {
    cancelConnection() {}, disconnect: () => new Promise<void>(resolve => {finish=resolve}),
    showConnections: async () => {}, connect: async () => {opened=true},
  });
  navigation.cancel(); finish(); await work;
  assert.equal(opened, false);
});

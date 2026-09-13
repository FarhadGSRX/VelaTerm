import { afterEach, expect, it, vi } from 'vitest';
const { invoke, notify } = vi.hoisted(() => ({invoke:vi.fn(),notify:vi.fn().mockResolvedValue(undefined)}));
vi.mock('../ipc/transport', () => ({invoke}));
vi.mock('../notify', () => ({notify}));
import { notifyMobileSession } from './notificationPreview';

afterEach(() => {vi.clearAllMocks();vi.useRealTimers()});
it('uses the host excerpt and retains the session destination', async () => {
  invoke.mockResolvedValue({title:'手机 App',body:'已修复通知点击后的会话跳转。'});
  await notifyMobileSession('session-2','Fallback title','Replied',true);
  expect(invoke).toHaveBeenCalledWith('mobile_notification_preview',{sessionId:'session-2'});
  expect(notify).toHaveBeenCalledWith('session-2','手机 App','已修复通知点击后的会话跳转。',true);
});
it('keeps the state notice when there is no reply or the host is old', async () => {
  invoke.mockResolvedValue({title:'Session',body:''});
  await notifyMobileSession('s','Old title','Needs confirmation',false);
  expect(notify).toHaveBeenLastCalledWith('s','Session','Needs confirmation',false);
  invoke.mockRejectedValue(new Error('Unknown command'));
  await notifyMobileSession('s','Old title','Replied',false);
  expect(notify).toHaveBeenLastCalledWith('s','Old title','Replied',false);
});
it('a stalled excerpt request cannot swallow the notification', async () => {
  vi.useFakeTimers();invoke.mockImplementation(() => new Promise(() => {}));
  const sent = notifyMobileSession('s','Session','Replied',false);
  await vi.advanceTimersByTimeAsync(3000);await sent;
  expect(notify).toHaveBeenCalledOnce();
});

import { invoke } from "../ipc/transport";
import { mobileNotifications } from "./nativeNotifications";

/** Bind only after authenticated project loading succeeds. Nothing is persisted in the WebView. */
export async function bindMobilePush(): Promise<void> {
  const native = mobileNotifications();
  if (!native?.getSubscription || !native.setBound) return;
  let subscription: {subscriptionId: string; publisherToken: string};
  try { subscription = await native.getSubscription() }
  catch { return }
  try {
    const result = await invoke<{active: boolean}>("mobile_push_bind", subscription);
    await native.setBound(result.active === true);
  } catch {
    // Older hosts and disabled providers retain local foreground delivery. Native settings show
    // the missing host binding; this must never make the project page unusable.
    await native.setBound(false).catch(() => {});
  }
}

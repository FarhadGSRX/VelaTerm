/** Narrow notification capability injected by the mobile host into the selected origin. */
interface MobileNotifications {
  getPermission(): Promise<"granted" | "denied" | "default" | "unsupported">;
  requestPermission(): ReturnType<MobileNotifications["getPermission"]>;
  send(payload: {sessionId: string | null; title: string; body: string; sound: boolean}): Promise<void>;
  getSubscription?(): Promise<{subscriptionId: string; publisherToken: string}>;
  setBound?(active: boolean): Promise<void>;
}
export function mobileNotifications(): MobileNotifications | undefined {
  return typeof window === "undefined" ? undefined : (window as unknown as {__VELATERM_NOTIFICATIONS__?: MobileNotifications}).__VELATERM_NOTIFICATIONS__;
}

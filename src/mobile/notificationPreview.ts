import { invoke } from "../ipc/transport";
import { notify } from "../notify";

/** The host chooses the current reply; unavailable/older hosts retain the original status notice. */
export async function notifyMobileSession(sessionId: string, title: string, body: string, sound: boolean): Promise<void> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  try {
    const preview = await Promise.race([
      invoke<{title: string; body: string}>("mobile_notification_preview", {sessionId}),
      new Promise<never>((_, reject) => { timer = setTimeout(() => reject(new Error("Preview unavailable")), 3000) }),
    ]);
    if (preview.title?.trim()) title = preview.title;
    if (preview.body?.trim()) body = preview.body;
  } catch { /* A missing excerpt must not suppress a task notification. */ }
  finally { clearTimeout(timer) }
  await notify(sessionId, title, body, sound);
}

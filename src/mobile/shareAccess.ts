//! What this connection may do with terminal-view sessions. A direct connection (URL or SSH) may do
//! everything; a share connection asks the host, which allows the live terminal and the view switch only
//! for the host's own account and re-checks both on every call.

import { useEffect, useState } from "react";
import { shareAccess, type ShareAccess } from "../ipc/chat";
import { isShareSurface } from "../ipc/shareBase";

const FULL: ShareAccess = { canSwitchEngine: true, canUseTerminal: true };
// A failed check or an older host that does not report terminal access leaves the visitor read-only.
const NONE: ShareAccess = { canSwitchEngine: false, canUseTerminal: false };

/** Null while a share connection is asking. */
export function useShareAccess(): ShareAccess | null {
  const [access, setAccess] = useState<ShareAccess | null>(isShareSurface ? null : FULL);
  useEffect(() => {
    if (!isShareSurface) return;
    let disposed = false;
    shareAccess().then(
      (a) => { if (!disposed) setAccess({ canSwitchEngine: !!a.canSwitchEngine, canUseTerminal: !!a.canUseTerminal }); },
      () => { if (!disposed) setAccess(NONE); },
    );
    return () => { disposed = true; };
  }, []);
  return access;
}

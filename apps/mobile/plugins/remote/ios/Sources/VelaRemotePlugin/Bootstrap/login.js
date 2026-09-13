// The native host injects this only into the selected service origin.
if (window === window.top) {
  const apple = window.webkit?.messageHandlers?.VelaPassword;
  const android = window.VelaPassword;
  if (apple || android) {
    let sequence = 0;
    const pending = new Map();
    if (!apple) android.onmessage = event => {
      try {
        const reply = JSON.parse(event.data);
        const request = pending.get(reply.id);
        if (!request) return;
        pending.delete(reply.id);
        clearTimeout(request.timer);
        reply.ok ? request.resolve() : request.reject(new Error('Password storage failed'));
      } catch { /* Ignore malformed replies without exposing credentials. */ }
    };
    window.__VELATERM_LOGIN__ = {
      savePassword(password) {
        if (apple) return apple.postMessage({password});
        return new Promise((resolve, reject) => {
          const id = ++sequence;
          const timer = setTimeout(() => {
            pending.delete(id);
            reject(new Error('Password storage timed out'));
          }, 10000);
          pending.set(id, {resolve, reject, timer});
          try { android.postMessage(JSON.stringify({id, password})); }
          catch { clearTimeout(timer); pending.delete(id); reject(new Error('Password storage unavailable')); }
        });
      },
      back() { window.location.assign('velaterm-ui://close'); },
    };
  }
}

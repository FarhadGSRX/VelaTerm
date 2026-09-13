// Exposed only to the selected service origin and its main frame by the native host.
if (window === window.top) {
  const apple = window.webkit?.messageHandlers?.VelaNotifications;
  const android = window.VelaNotifications;
  if (apple || android) {
    let sequence = 0;
    const pending = new Map();
    if (!apple) android.onmessage = event => {
      try {
        const reply = JSON.parse(event.data);
        const request = pending.get(reply.id);
        if (!request) return;
        pending.delete(reply.id); clearTimeout(request.timer);
        reply.error ? request.reject(new Error(reply.error)) : request.resolve(reply.result);
      } catch { /* Ignore malformed native replies. */ }
    };
    const call = payload => {
      if (apple) return apple.postMessage(payload);
      return new Promise((resolve, reject) => {
        const id = ++sequence;
        const timer = setTimeout(() => { pending.delete(id); reject(new Error('Notification request timed out')); }, 60000);
        pending.set(id, {resolve, reject, timer});
        try { android.postMessage(JSON.stringify({...payload, id})); }
        catch { clearTimeout(timer); pending.delete(id); reject(new Error('Notifications unavailable')); }
      });
    };
    window.__VELATERM_NOTIFICATIONS__ = {
      getPermission: () => call({action: 'permission', request: false}),
      requestPermission: () => call({action: 'permission', request: true}),
      send: payload => call({...payload, action: 'send'}),
      getSubscription: () => call({action: 'subscription'}),
      setBound: active => call({action: 'bound', active}),
    };
  }
}

// HTML completion is not application readiness. Keep native navigation until an actual UI is painted.
if (window === window.top) {
  const apple = window.webkit?.messageHandlers?.VelaPageReady;
  const android = window.VelaPageReady;
  if (apple || android) {
    let finished = false;
    let scheduled = false;
    const visible = element => {
      if (!element) return false;
      const rect = element.getBoundingClientRect();
      const style = getComputedStyle(element);
      return rect.width > 0 && rect.height > 0 && style.display !== 'none' && style.visibility !== 'hidden';
    };
    const ready = () => {
      if (document.readyState === 'loading') return false;
      if ([...document.querySelectorAll('link[rel="stylesheet"]')].some(link => !link.disabled && !link.sheet)) return false;
      const login = document.querySelector('.login-screen');
      if (login) return visible(login) && !!login.querySelector('button, input, a[href]');
      const mobile = document.querySelector('.m-app');
      if (mobile) return visible(mobile.querySelector('.m-header')) && !mobile.querySelector('.m-load-status, .m-list[aria-busy="true"]');
      // Older project pages still need a rendered application and usable controls.
      const root = document.querySelector('#root');
      return visible(root) && !!root.querySelector('button, input, a[href]') && !!root.textContent.trim();
    };
    const stop = () => { finished = true; observer.disconnect(); window.removeEventListener('load', check); };
    const check = () => {
      if (finished || scheduled) return;
      scheduled = true;
      requestAnimationFrame(() => requestAnimationFrame(() => {
        scheduled = false;
        if (finished || !ready()) return;
        stop();
        try {
          if (apple) apple.postMessage({ready: true}).catch(() => {});
          else android.postMessage(JSON.stringify({ready: true}));
        } catch { /* The native loading page remains available if the bridge has closed. */ }
      }));
    };
    const observer = new MutationObserver(check);
    observer.observe(document, {subtree: true, childList: true, attributes: true});
    window.addEventListener('load', check);
    window.addEventListener('pagehide', stop, {once: true});
    check();
  }
}

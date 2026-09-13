// The isolated project WebView exposes only a user-confirmed file export bridge.
(() => {
  let pending = false;
  const limit = 64 * 1024 * 1024;
  const save = async (address, name) => {
    if (pending) return;
    const url = new URL(address, location.href);
    if (!['blob:', 'data:'].includes(url.protocol) && url.origin !== location.origin) return;
    pending = true;
    try {
      const headers = {};
      const token = sessionStorage.getItem('vlx-token');
      if (url.origin === location.origin && ['http:', 'https:'].includes(url.protocol) && token) headers.Authorization = `Bearer ${token}`;
      const response = await fetch(url.href, { headers });
      if (!response.ok) throw new Error('下载失败，请重试');
      if (Number(response.headers.get('Content-Length')) > limit) throw new Error('手机文件导出目前支持不超过 64 MB 的文件');
      const reader = response.body.getReader();
      const chunks = [];
      let total = 0;
      while (true) {
        const { value, done } = await reader.read();
        if (done) break;
        total += value.byteLength;
        if (total > limit) { await reader.cancel(); throw new Error('手机文件导出目前支持不超过 64 MB 的文件'); }
        chunks.push(value);
      }
      const blob = new Blob(chunks);
      const base64 = await new Promise((resolve, reject) => {
        const reader = new FileReader();
        reader.onload = () => resolve(String(reader.result).split(',')[1]);
        reader.onerror = reject;
        reader.readAsDataURL(blob);
      });
      window.VelaFileExport.save(name || 'download', base64);
    } catch (error) { alert(error instanceof Error ? error.message : '下载失败，请重试'); }
    finally { pending = false; }
  };
  window.__velaSaveDownload = save;
  document.addEventListener('click', event => {
    const anchor = event.target instanceof Element ? event.target.closest('a[download]') : null;
    if (!anchor) return;
    event.preventDefault();
    void save(anchor.href, anchor.download);
  }, true);
})();

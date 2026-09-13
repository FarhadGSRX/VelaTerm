import { uploadDocImage } from "../../../ipc/transport";
import { loadFileBlob } from "../../../ipc/info";
import { extOf } from "../../../terminal/imageInput";

/** Map extensions to MIME types so object URLs render formats such as SVG correctly. */
const IMG_MIME: Record<string, string> = {
  png: "image/png",
  jpg: "image/jpeg",
  jpeg: "image/jpeg",
  gif: "image/gif",
  webp: "image/webp",
  bmp: "image/bmp",
  svg: "image/svg+xml",
  tiff: "image/tiff",
};
function mimeFromPath(p: string): string {
  const ext = p.split(".").pop()?.toLowerCase() ?? "";
  return IMG_MIME[ext] ?? "application/octet-stream";
}

/** Reuse object URLs by absolute path to avoid repeated local reads and memory leaks. */
const docImgUrlCache = new Map<string, string>();

/** Save pasted/dropped images under a sibling assets directory and return the Markdown-relative path. */
export async function onUploadDocImage(file: File, docPath: string): Promise<string> {
  if (!docPath) {
    return new Promise((resolve, reject) => {
      const reader = new FileReader();
      reader.onload = () => typeof reader.result === "string" ? resolve(reader.result) : reject(new Error("Image encoding failed"));
      reader.onerror = () => reject(reader.error ?? new Error("Image read failed"));
      reader.readAsDataURL(file);
    });
  }
  const bytes = new Uint8Array(await file.arrayBuffer());
  return uploadDocImage(bytes, extOf(file), docPath);
}

/**
 * Convert Markdown image sources into WebView-displayable URLs:
 * - Preserve HTTP, HTTPS, data, and blob sources.
 * - Resolve local relative paths against the document, then use loadFileBlob to build object URLs.
 *   This reuses ImageDocView's chunked I/O and avoids changing Tauri asset-protocol security settings.
 *
 * On read failure, return the original source for Crepe's placeholder/failure handling without throwing.
 */
export async function proxyDocImageURL(src: string, docPath: string): Promise<string> {
  if (/^(https?:|data:|blob:)/i.test(src)) return src;
  const docDir = docPath.replace(/[/\\][^/\\]*$/, "");
  const isAbs = src.startsWith("/") || /^[a-zA-Z]:[/\\]/.test(src);
  const abs = isAbs ? src : docDir ? `${docDir}/${src}` : src;
  const cached = docImgUrlCache.get(abs);
  if (cached) return cached;
  try {
    const res = await loadFileBlob(abs);
    if (!res) return src;
    const url = URL.createObjectURL(
      new Blob([res.blob], { type: mimeFromPath(abs) }),
    );
    docImgUrlCache.set(abs, url);
    return url;
  } catch {
    return src;
  }
}


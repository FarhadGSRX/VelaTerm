import type { AnchorHTMLAttributes } from "react";
import { memoryNavigate, useMemoryLocation } from "../layout/Memory/navigation";
export { useMemoryLocation as useSharingLocation, memoryNavigate as sharingNavigate };
export function sharingUrl(values: Record<string, string | null> = {}) {
  const url = new URL(location.href);
  for (const key of ["publicAccount", "sharedProject", "sharedSession", "shareTarget"]) url.searchParams.delete(key);
  for (const [key, value] of Object.entries(values)) { if (value) url.searchParams.set(key, value); else url.searchParams.delete(key); }
  return url.href;
}
export function SharingLink({ values, ...props }: AnchorHTMLAttributes<HTMLAnchorElement> & { values?: Record<string, string | null> }) {
  const href = sharingUrl(values);
  return <a {...props} href={href} onClick={e => {
    props.onClick?.(e);
    if (e.defaultPrevented || e.button !== 0 || e.metaKey || e.ctrlKey || e.altKey || e.shiftKey) return;
    e.preventDefault(); memoryNavigate(href);
  }} />;
}

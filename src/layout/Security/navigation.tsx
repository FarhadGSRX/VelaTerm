import type { AnchorHTMLAttributes } from "react";
import { memoryNavigate } from "../Memory/navigation";
export function securityUrl(projectId: string, values: Record<string,string|null> = {}) {
  const url=new URL(window.location.href);
  for(const key of [...url.searchParams.keys()]) {
    if (key.startsWith("security") && (!projectId || url.searchParams.get("security")!==projectId)) url.searchParams.delete(key);
    if(projectId && (key.startsWith("knowledge") || key.startsWith("memory"))) url.searchParams.delete(key);
  }
  if(projectId) url.searchParams.set("security",projectId);
  for(const [key,value] of Object.entries(values)) {if(value) url.searchParams.set(key,value);else url.searchParams.delete(key);}
  return url.href;
}
export function AuditLink({projectId,values,...props}: AnchorHTMLAttributes<HTMLAnchorElement> & {projectId:string;values?:Record<string,string|null>}) {
  const href=securityUrl(projectId,values);
  return <a {...props} href={href} onClick={(e)=>{props.onClick?.(e);if(e.defaultPrevented||e.metaKey||e.ctrlKey||e.shiftKey||e.altKey||e.button!==0)return;e.preventDefault();memoryNavigate(href);}}/>;
}

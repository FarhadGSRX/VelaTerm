/** Stable conversation URLs within the current remote grant. */
export function sharedSessionUrl(id:string|null):string {
  const url=new URL(window.location.href);
  if(id)url.searchParams.set("session",id);else url.searchParams.delete("session");
  return url.pathname+url.search+url.hash;
}
export function selectedSharedSession():string|null {return new URLSearchParams(window.location.search).get("session")}
export function navigateSharedSession(id:string|null) {
  const url=sharedSessionUrl(id);
  if(url===location.pathname+location.search+location.hash)return;
  history.pushState(null,"",url);window.dispatchEvent(new PopStateEvent("popstate"));
}

export type Route = {page:'list'} | {page:'remote'} | {page:'account'} | {page:'notifications'} | {page:'edit'; mode:'url'|'ssh'; id?:string; copyFromId?:string; scan?:true};
export function routeFromHash(hash:string):Route {
  const url=new URL(hash.replace(/^#/,'') || '/', 'https://app.local');
  if(url.pathname==='/account') return {page:'account'};
  if(url.pathname==='/remote') return {page:'remote'};
  if(url.pathname==='/notifications') return {page:'notifications'};
  if(url.pathname==='/connections/new') return {page:'edit',mode:url.searchParams.get('mode')==='ssh'?'ssh':'url',...(url.searchParams.get('mode')!=='ssh' && url.searchParams.get('scan')==='1'?{scan:true as const}:{})};
  const match=/^\/connections\/([^/]+)\/edit$/.exec(url.pathname);
  if(match) return {page:'edit',mode:'url',id:decodeURIComponent(match[1])};
  const copy=/^\/connections\/([^/]+)\/copy$/.exec(url.pathname);
  if(copy) return {page:'edit',mode:'url',copyFromId:decodeURIComponent(copy[1])};
  return {page:'list'};
}
export function editHref(id:string):string {return '#/connections/'+encodeURIComponent(id)+'/edit'}
export function copyHref(id:string):string {return '#/connections/'+encodeURIComponent(id)+'/copy'}

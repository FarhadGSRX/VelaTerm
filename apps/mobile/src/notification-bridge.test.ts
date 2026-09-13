import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {runInNewContext} from 'node:vm';

const source=readFileSync(new URL('../plugins/remote/shared/notifications.js',import.meta.url),'utf8');
function load(native:Record<string,unknown>,mainFrame=true) {
  const window:Record<string,any>={...native};window.top=mainFrame?window:{};
  runInNewContext(source,{window,setTimeout,clearTimeout});
  return window.__VELATERM_NOTIFICATIONS__;
}
test('iOS distinguishes permission checks from prompts and awaits notification delivery',async()=>{
  const calls:any[]=[];
  const bridge=load({webkit:{messageHandlers:{VelaNotifications:{postMessage:async(payload:any)=>{
    calls.push(payload);return payload.action==='permission'?'granted':true;
  }}}}});
  assert.equal(await bridge.getPermission(),'granted');
  assert.equal(await bridge.requestPermission(),'granted');
  await bridge.send({sessionId:'session',title:'Fixture',body:'Complete',sound:true});
  assert.equal(calls[0].request,false);assert.equal(calls[1].request,true);
  assert.equal(calls[2].action,'send');assert.equal(calls[2].sessionId,'session');
});
test('Android correlates permission and delivery replies and surfaces denied delivery',async()=>{
  const calls:any[]=[];const native:any={postMessage:(raw:string)=>calls.push(JSON.parse(raw))};
  const bridge=load({VelaNotifications:native});
  const permission=bridge.getPermission();const delivery=bridge.send({sessionId:'session',title:'Fixture',body:'Complete',sound:true});
  native.onmessage({data:JSON.stringify({id:calls[1].id,error:'Notifications unavailable'})});
  native.onmessage({data:JSON.stringify({id:calls[0].id,result:'denied'})});
  assert.equal(await permission,'denied');await assert.rejects(delivery,/Notifications unavailable/);
  native.onmessage({data:'malformed'});
});
test('ordinary web pages and subframes cannot access the notification capability',()=>{
  assert.equal(load({}),undefined);
  assert.equal(load({VelaNotifications:{}},false),undefined);
});
test('background subscription returns only the native capability and requires explicit host acknowledgement',async()=>{
  const calls:any[]=[];
  const bridge=load({webkit:{messageHandlers:{VelaNotifications:{postMessage:async(payload:any)=>{
    calls.push(payload);
    return payload.action==='subscription'?{subscriptionId:'sub',publisherToken:'capability'}:true;
  }}}}});
  assert.deepEqual(JSON.parse(JSON.stringify(await bridge.getSubscription())),{subscriptionId:'sub',publisherToken:'capability'});
  await bridge.setBound(true);await bridge.setBound(false);
  assert.deepEqual(calls.map(item=>JSON.parse(JSON.stringify(item))),[{action:'subscription'},{action:'bound',active:true},{action:'bound',active:false}]);
});

import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {runInNewContext} from 'node:vm';

const source=readFileSync(new URL('../plugins/remote/shared/login.js',import.meta.url),'utf8');
function load(native:Record<string,unknown>, mainFrame=true) {
  const sent:string[]=[];
  const window:Record<string,any>={...native,location:{assign:(url:string)=>sent.push(url)}};
  window.top=mainFrame?window:{};
  runInNewContext(source,{window,setTimeout,clearTimeout});
  return {window,sent};
}
test('iOS awaits native storage acknowledgement and returns through the close command', async()=>{
  const passwords:string[]=[];
  const {window,sent}=load({webkit:{messageHandlers:{VelaPassword:{postMessage:async({password}:{password:string})=>{passwords.push(password)}}}}});
  await window.__VELATERM_LOGIN__.savePassword('fixture-only');
  assert.deepEqual(passwords,['fixture-only']);
  window.__VELATERM_LOGIN__.back();
  assert.deepEqual(sent,['velaterm-ui://close']);
});
test('Android correlates replies and surfaces failed native writes', async()=>{
  const requests:any[]=[];
  const native:any={postMessage:(value:string)=>requests.push(JSON.parse(value))};
  const {window}=load({VelaPassword:native});
  const first=window.__VELATERM_LOGIN__.savePassword('fixture-only');
  native.onmessage({data:JSON.stringify({id:requests[0].id,ok:true})});
  await first;
  const second=window.__VELATERM_LOGIN__.savePassword('');
  native.onmessage({data:JSON.stringify({id:requests[1].id,ok:false})});
  await assert.rejects(second,/storage failed/);
  assert.notEqual(requests[0].id,requests[1].id);
});
test('does not expose password storage to child frames or an ordinary browser',()=>{
  assert.equal(load({VelaPassword:{}},false).window.__VELATERM_LOGIN__,undefined);
  assert.equal(load({}).window.__VELATERM_LOGIN__,undefined);
});

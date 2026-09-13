import {test} from 'node:test';
import assert from 'node:assert/strict';
import {AccountLogin} from './account-login.ts';

test('authorization survives a temporary network failure without creating another device request',async()=>{
  const actions:string[]=[];let polls=0;
  const login=new AccountLogin(async action=>{
    actions.push(action);
    if(action==='login')return {};
    if(++polls===1)throw new Error('offline');
    return {linked:true};
  },60000);
  try {
    await login.start();assert.equal(login.state.phase,'retrying');
    await login.check();assert.equal(login.state.phase,'linked');
    assert.deepEqual(actions,['login','poll','poll']);
  }finally{login.reset()}
});

test('remounting the account page resumes the native pending request without starting a new login',async()=>{
  const actions:string[]=[];
  const login=new AccountLogin(async action=>{actions.push(action);return {linked:true}});
  const messages:string[]=[];const unmount=login.subscribe(state=>messages.push(state.phase));unmount();
  await login.resume();
  assert.deepEqual(actions,['poll']);assert.equal(login.state.phase,'linked');assert.deepEqual(messages,[]);
});

test('returning from the browser does not issue concurrent polls',async()=>{
  let resolve!:(value:{linked:boolean})=>void;let calls=0;
  const login=new AccountLogin(async()=>{calls++;return new Promise(done=>{resolve=done})});
  const pending=login.resume();await login.check();await login.resume();assert.equal(calls,1);
  resolve({linked:true});await pending;assert.equal(login.state.phase,'linked');
});

test('server expiry stops polling and allows a fresh login',async()=>{
  const login=new AccountLogin(async action=>{
    if(action==='login')return {};
    throw Object.assign(new Error('expired'),{code:'ACCOUNT_LOGIN_EXPIRED'});
  });
  await login.start();assert.equal(login.state.phase,'error');assert.equal(login.active,false);
});

test('a response from an abandoned login cannot overwrite the next account state',async()=>{
  let resolve!:(value:{linked:boolean})=>void;
  const login=new AccountLogin(async()=>new Promise(done=>{resolve=done}));
  const pending=login.resume();login.reset();resolve({linked:true});await pending;
  assert.equal(login.state.phase,'idle');
});

import { Capacitor } from '@capacitor/core';
import { Remote, type Connection } from './remote';
import { routeFromHash, editHref, copyHref } from './routes';
import { initI18n, t } from '../../../src/i18n';
import { AccountLogin } from './account-login';
import { ConnectionAttempt } from './connection-attempt';
import { notificationSettings } from './notification-settings';
import { NotificationNavigation } from './notification-navigation';
import './style.css';
const app=document.querySelector<HTMLElement>('#app')!;
let records:Connection[]=[];
let busy=false;
let scanning=false;
const accountLogin=new AccountLogin(action=>Remote.account({action}));
let releaseLoginView:()=>void=()=>{};
let renderGeneration=0;
const phases:Record<string,string>={connecting:'正在连接 SSH…',confirming:'请确认主机指纹',preparing:'正在检查或准备远端服务…',forwarding:'正在建立 SSH 隧道…',ready:'连接已建立',disconnected:'连接已断开',error:'连接失败'};
function el<K extends keyof HTMLElementTagNameMap>(tag:K,text?:string,cls?:string):HTMLElementTagNameMap[K] {const e=document.createElement(tag);if(text)e.textContent=text;if(cls)e.className=cls;return e}
function link(text:string,href:string):HTMLAnchorElement {const a=el('a',text);a.href=href;return a}
function button(text:string,action:()=>void,cls=''):HTMLButtonElement {const b=el('button',text,cls);b.type='button';b.onclick=action;return b}
function status(text:string,error=false) {const target=document.querySelector<HTMLElement>('.connection-loading [role=status]') ?? document.querySelector<HTMLElement>('[role=status]');if(target){target.textContent=text;target.classList.toggle('error',error)}}
function lock(value:boolean) {busy=value;document.querySelectorAll<HTMLButtonElement>('button[data-connect],button[type=submit]').forEach(b=>b.disabled=value)}
const connectionAttempt = new ConnectionAttempt();
const notificationNavigation = new NotificationNavigation();
let releaseConnectionLoading: () => void = () => {};
function cancelConnection() {
  notificationNavigation.cancel();
  connectionAttempt.cancel(); releaseConnectionLoading(); lock(false);
  void Remote.disconnect().catch(error => status(String(error), true));
}
async function connect(id:string,sessionId?:string) {
  if(busy || scanning)return;
  lock(true);
  const panel = el('dialog', undefined, 'connection-loading');
  panel.setAttribute('aria-labelledby', 'connection-loading-title');
  const content = el('div', undefined, 'connection-loading-content');
  const progress = el('span', undefined, 'connection-spinner'); progress.setAttribute('aria-hidden', 'true');
  const title = el('h2', t('common.loading')); title.id = 'connection-loading-title';
  const detail = el('p'); detail.setAttribute('role', 'status'); detail.setAttribute('aria-live', 'polite');
  const back = button(t('mobile.backConnections'), cancelConnection);
  back.id = 'connection-back';
  content.append(progress, title, detail, back); panel.append(content); document.body.append(panel);
  panel.addEventListener('cancel', event => { event.preventDefault(); cancelConnection() });
  const timer = window.setTimeout(() => { detail.textContent = t('mobile.loadSlow') }, 30000);
  releaseConnectionLoading = () => { clearTimeout(timer); panel.close(); panel.remove(); releaseConnectionLoading = () => {} };
  panel.showModal();
  await connectionAttempt.run(
    () => id.startsWith('account_')
      ? Remote.account({action:'open',grantId:id.slice(8),...(sessionId?{sessionId}:{})})
      : Remote.connect({id,...(sessionId?{sessionId}:{})}),
    error => { progress.hidden = true; title.textContent = t('mobile.connectionUnavailable'); detail.textContent = error instanceof Error ? error.message : String(error); clearTimeout(timer) },
    () => { if (!progress.hidden) { releaseConnectionLoading(); lock(false) } },
  );
}
function field(form:HTMLFormElement,label:string,name:string,value='',type='text',placeholder='') {
  const wrap=el('label',label);const input=el('input');input.name=name;input.type=type;input.value=value;input.placeholder=placeholder;input.autocomplete='off';input.spellcheck=false;input.setAttribute('autocapitalize','none');wrap.append(input);form.append(wrap);return input;
}
function select(form:HTMLFormElement,label:string,name:string,options:[string,string][],value:string) {
  const wrap=el('label',label);const s=el('select');s.name=name;for(const [key,text] of options){const o=el('option',text);o.value=key;s.append(o)}s.value=value;wrap.append(s);form.append(wrap);return s;
}
function check(form:HTMLFormElement,label:string,name:string,value:boolean) {const wrap=el('label',undefined,'check');const input=el('input');input.type='checkbox';input.name=name;input.checked=value;wrap.append(input,document.createTextNode(label));form.append(wrap);return input}
async function render() {
  const generation=++renderGeneration;
  releaseLoginView();releaseLoginView=()=>{};
  if(busy) cancelConnection();
  app.replaceChildren();const header=el('header');const logo=el('img',undefined,'mark');logo.src=new URL('../assets/icon-ios.svg',import.meta.url).href;logo.alt='';logo.width=44;logo.height=44;logo.draggable=false;header.append(logo,el('div','VelaTerm','brand'));const accountButton=link('','#/account');accountButton.className='account-button';accountButton.innerHTML='<svg width=22 height=22 viewBox="0 0 24 24" fill=none stroke=currentColor stroke-width=1.7 aria-hidden=true><circle cx=12 cy=8 r=4></circle><path d="M4 22v-2a8 8 0 0 1 16 0v2"></path></svg>';accountButton.title='账号与登录';accountButton.setAttribute('aria-label','账号与登录');header.append(accountButton);app.append(header);
  const notice=el('p');notice.setAttribute('role','status');notice.setAttribute('aria-live','polite');
  try {records=(await Remote.list()).connections} catch(error) {app.append(el('h1','连接服务'),el('p',Capacitor.isNativePlatform()?String(error):'请在 iOS 或 Android App 中使用连接功能。浏览器仅用于检查界面。'));records=[]}
  if(generation!==renderGeneration)return;
  const footer=el('footer');footer.append(notice,el('p','项目与会话由远端服务管理。','hint'),el('p',`App v${__MOBILE_VERSION__} · 构建 ${__MOBILE_BUILD_TIME__}（北京时间）`,'hint build-info'));
  const route=routeFromHash(location.hash);
  if(route.page==='notifications') {
    app.append(link('‹ '+t('mobile.backConnections'),'#/'),el('h1',t('mobile.pushTitle')),notificationSettings(records));
  } else if(route.page==='remote' || route.page==='account') {
    app.append(link('‹ 连接列表','#/'),el('h1',route.page==='remote'?'我的设备':'账号'));
    const panel=el('section',undefined,'remote-account-card');app.append(panel);
    if(route.page==='account') app.append(link(t('mobile.pushTitle'),'#/notifications'));
    let linked=false;let accountKnown=false;let pending=false;
    try {
      const result=await Remote.account({action:'status'});
      if(generation!==renderGeneration)return;
      linked=!!result.linked;pending=!!result.pending;accountKnown=true;
      if(linked) {
        panel.append(el('h2',result.account?.displayName ?? 'VelaTerm'),el('p','已登录。可查看同账号设备共享的空间、项目和会话。','muted'));
        if(route.page==='account') panel.append(button('管理账号',()=>void Remote.account({action:'open'}).catch(e=>status(String(e),true))),button('退出登录',()=>void Remote.account({action:'logout'}).then(()=>{accountLogin.reset();return render()}).catch(e=>status(String(e),true))),link('查看我的设备','#/remote'));
        else {
          const list=el('div',undefined,'remote-client-list');app.append(list);
          const refresh=async()=>{
            try {
              const {devices=[]}=await Remote.account({action:'devices'});
              if(!list.isConnected)return;list.replaceChildren();
              if(!devices.length)list.append(el('p','此账号尚未登录任何设备。'));
              for(const device of devices){
                const card=el('section',undefined,'remote-client');card.append(el('h2',device.name),el('p',device.online?'在线':'离线','muted'));
                if(!device.access.length){card.append(el('p','此设备尚未共享内容。','muted'));list.append(card);continue;}
                const scopes=el('ul',undefined,'remote-client-shares');
                for(const scope of device.access)scopes.append(el('li',scope.scope==='machine'?'整个工作空间':scope.name===scope.scope?(scope.scope==='project'?'项目':'会话'):scope.name));
                card.append(scopes);
                if(!device.online || !device.sharing){card.append(el('p',device.online?'共享内容尚未就绪，请在该设备上检查共享设置。':'设备已离线，请在该设备上打开 VelaTerm 并保持网络连接。','muted'));list.append(card);continue;}
                const open=button('查看共享内容 →',async()=>{
                  open.disabled=true;
                  try{await Remote.account({action:'open',deviceId:device.id})}catch(error){status(String(error),true)}finally{open.disabled=false}
                });
                card.append(open);list.append(card);
              }
            }catch(error){if(list.isConnected){list.replaceChildren(el('p','无法获取设备列表，请重试。'));status(String(error),true);}}
            if(list.isConnected)setTimeout(()=>void refresh(),5000);
          };void refresh();
        }
      }
    }catch(error){panel.append(el('p','无法获取账号状态，请检查网络后重试。'),button('重试',()=>void render()));notice.textContent=String(error);notice.classList.add('error');}
    if(!linked && accountKnown) {
      panel.append(el('h2','登录 VelaTerm'),el('p','通过邮箱密码或第三方账号登录，查看你的设备和共享内容。'));
      const signIn=button('登录',()=>void accountLogin.start(),'primary');
      const progress=el('p');progress.setAttribute('role','status');progress.setAttribute('aria-live','polite');
      const checkLogin=button('检查登录结果',()=>void accountLogin.check());
      panel.append(signIn,progress,checkLogin);
      const update=()=>{
        signIn.disabled=accountLogin.active;signIn.textContent=accountLogin.active?'正在等待登录确认…':'登录';
        progress.textContent=accountLogin.state.message;progress.hidden=!progress.textContent;
        progress.classList.toggle('error',accountLogin.state.phase==='error');checkLogin.hidden=!accountLogin.active;
        if(accountLogin.state.phase==='linked')void render();
      };
      releaseLoginView=accountLogin.subscribe(update);
      if(!pending && !accountLogin.active)accountLogin.reset();
      update();if(pending)void accountLogin.resume();
    }
  } else if(route.page==='list') {
    app.append(el('h1','你的工作空间'),el('p','连接远程主机，继续项目中的工作。','muted'));
    const actions=el('nav',undefined,'actions');actions.append(link('＋ SSH 连接','#/connections/new?mode=ssh'),link('＋ URL 连接','#/connections/new?mode=url'),link('Remote','#/remote'),link('扫码连接','#/connections/new?mode=url&scan=1'));app.append(actions);
    if(!records.length) app.append(el('section','尚未保存连接。可添加 SSH、URL 连接，或通过 Remote 查看同账号设备的共享内容。','empty'));
    for(const row of records) {
      const card=el('article',undefined,'connection-card');
      const open=button('',()=>void connect(row.id!),'connection-open');open.dataset.connect='true';
      let address=row.mode==='ssh'?`${row.username}@${row.host}:${row.port}`:row.url ?? '';
      if(row.mode==='url'){try{const url=new URL(address);address=url.origin+url.pathname}catch{/* 保留无法解析的旧地址供用户编辑。 */}}
      open.append(el('span',row.name,'connection-name'),el('span',address,'connection-address muted'),el('span','点击连接 →','connection-action'));
      card.append(open);
      if(row.hasWebPassword)card.append(el('p','服务密码已保存','saved-secret hint'));
      const controls=el('div',undefined,'controls');controls.append(el('span',row.mode.toUpperCase(),'badge'),link('编辑',editHref(row.id!)),link(t('mobile.copyConnection'),copyHref(row.id!)),button('删除',()=>{
        const dialog=el('dialog');dialog.append(el('h2','删除连接'),el('p',`删除“${row.name}”及其保存的凭据？远端项目不会被删除。`),button('取消',()=>dialog.remove()),button('删除',()=>{void Remote.remove({id:row.id!}).then(()=>{dialog.remove();return render()}).catch(e=>status(String(e),true))}));app.append(dialog);dialog.showModal();
      }));card.append(controls);app.append(card);
    }
  } else {
    const sourceId=route.copyFromId ?? route.id;
    const row=records.find(r=>r.id===sourceId);if(sourceId && !row){app.append(el('p','连接不存在'),link('返回连接列表','#/'));return}
    const mode=row?.mode ?? route.mode;
    app.append(link('‹ 连接列表','#/'),el('h1',route.copyFromId?t('mobile.copyConnection'):row?'编辑连接':mode==='ssh'?'添加 SSH 主机':'添加 URL 连接'));
    if(route.copyFromId)app.append(el('p',t('mobile.copyConnectionHint'),'hint'));
    const form=el('form');const nameInput=field(form,'连接名称','name',row?.name);nameInput.required=true;
    if(mode==='url') {
      const urlInput=field(form,'服务地址','url',row?.url,'url','https://your-server.example');urlInput.required=true;
      const scan=button('扫码填写',()=>void scanURL());scan.className='scan-button';form.append(scan);
      async function scanURL() {
        if(scanning || busy)return;
        scanning=true;scan.disabled=true;form.querySelectorAll<HTMLButtonElement>('button[type=submit]').forEach(b=>b.disabled=true);
        status('正在打开相机…');
        try {
          const result=await Remote.scanURL();
          if(!form.isConnected)return;
          if('cancelled' in result){status('已取消扫码');return}
          urlInput.value=result.url;if(!nameInput.value.trim())nameInput.value=result.name;
          status('已识别服务地址，请确认后保存并连接。');
        } catch(error) {if(form.isConnected)status(Capacitor.isNativePlatform()?String(error):'请在手机 App 中使用相机扫码。',true)}
        finally {scanning=false;scan.disabled=false;form.querySelectorAll<HTMLButtonElement>('button[type=submit]').forEach(b=>b.disabled=busy)}
      }
      if(route.scan) {
        history.replaceState(null,'','#/connections/new?mode=url');
        setTimeout(()=>{if(form.isConnected)void scanURL()},0);
      }
      field(form,'服务密码（可选）','webPassword','','password',row?'留空保留原密码':'也可进入网页后登录');
      form.append(el('p',row?.hasWebPassword?'服务密码已保存，重新连接时会自动使用。留空不会清除已保存的密码。':'密码保存在手机安全存储中，也可在登录时选择记住密码。','hint'));
    } else {
      field(form,'SSH 主机','host',row?.host,'text','主机名或 IP 地址').required=true;
      field(form,'SSH 端口','port',String(row?.port ?? 22),'number').required=true;
      field(form,'用户名','username',row?.username).required=true;
      const auth=select(form,'认证方式','auth',[['password','密码'],['key',Capacitor.getPlatform()==='android'?'私钥（OpenSSH Ed25519 / RSA）':'私钥（OpenSSH Ed25519）']],row?.auth ?? 'password');
      const password=field(form,'SSH 密码','password','','password',row?'留空保留原密码':'');
      const keyWrap=el('label','私钥');const key=el('textarea');key.name='privateKey';key.rows=6;key.spellcheck=false;key.placeholder=row?'留空保留已保存私钥':'粘贴 OpenSSH 私钥';keyWrap.append(key);form.append(keyWrap);
      const pass=field(form,'私钥口令（可选）','passphrase','','password',row?'留空保留原口令':'');
      if(row?.hasSecret)form.append(el('p','SSH 凭据已保存在手机安全存储中，编辑时留空即可保留。','hint'));
      const updateAuth=()=>{password.parentElement!.hidden=auth.value!=='password';keyWrap.hidden=pass.parentElement!.hidden=auth.value!=='key'};auth.onchange=updateAuth;updateAuth();
      const service=select(form,'远端服务','service',[['auto','自动查找 VelaTerm 服务'],['manual','指定已有服务端口']],row?.service ?? 'auto');
      const remotePort=field(form,'远端回环 HTTP 服务端口','remotePort',row?.remotePort?String(row.remotePort):'','number');
      const webPassword=field(form,'服务密码（可选）','webPassword','','password',row?'留空保留原密码':'');
      if(row?.hasWebPassword)form.append(el('p','服务密码已保存，重新连接时会自动使用。','hint'));
      const prepare=check(form,'没有可用服务时，允许下载并启动 VelaTerm 服务','prepare',row?.prepare ?? false);
      const detail=el('p','自动准备会在远端 ~/.velaterm/ 写入经过签名校验的程序、配置和日志，并保留运行服务。需要 Python 3 和支持 Ed25519 的 OpenSSL；复用已有服务或指定端口不需要安装这些工具。','hint');form.append(detail);
      const updateService=()=>{remotePort.parentElement!.hidden=webPassword.parentElement!.hidden=service.value!=='manual';prepare.parentElement!.hidden=detail.hidden=service.value!=='auto'};service.onchange=updateService;updateService();
    }
    const saves=el('div',undefined,'save-actions');
    const save=el('button','保存连接');save.type='submit';saves.append(save);
    if(mode==='url'){const open=el('button','保存并连接','primary');open.type='submit';open.value='connect';saves.append(open)}
    if(route.copyFromId)saves.append(link(t('common.cancel'),'#/'));
    form.append(saves);
    form.onsubmit=e=>{e.preventDefault();if(busy || scanning)return;const openAfter=(e.submitter as HTMLButtonElement | null)?.value==='connect';const values=new FormData(form);const data:Connection={...(!route.copyFromId && row?{id:row.id}:{}),name:String(values.get('name')??'').trim(),mode};
      if(mode==='url') data.url=String(values.get('url')).trim();
      else Object.assign(data,{host:String(values.get('host')).trim(),port:Number(values.get('port')),username:String(values.get('username')).trim(),auth:values.get('auth'),service:values.get('service'),remotePort:Number(values.get('remotePort')),prepare:values.get('prepare')==='on'});
      for(const name of ['password','privateKey','passphrase','webPassword'] as const){const value=String(values.get(name)??'');if(value || !row)data[name]=value}
      lock(true);void Remote.save({connection:data,...(route.copyFromId?{copyFromId:route.copyFromId}:{})}).then(async result=>{lock(false);history.pushState(null,'','#/');await render();if(openAfter)await connect(result.connection.id!);else if(route.copyFromId && records.some(r=>r.id===result.connection.id && r.id===sourceId))status(t('mobile.copyConnectionReused'))}).catch(e=>{lock(false);status(String(e),true)});
    };app.append(form);
  }
  if(generation===renderGeneration)app.append(footer);
}
window.addEventListener('hashchange',()=>void render());
document.addEventListener('visibilitychange',()=>{if(!document.hidden)void accountLogin.check()});
window.addEventListener('focus',()=>void accountLogin.check());
if(Capacitor.isNativePlatform()) void Remote.addListener('state',event=>{
  if(event.phase==='disconnected' && !busy && !scanning && routeFromHash(location.hash).page==='list')void render().then(()=>status(phases.disconnected));
  else status(phases[event.phase]??event.phase,event.phase==='error');
});
const initialized = initI18n().then(render);
if(Capacitor.isNativePlatform()) void Remote.addListener('notificationOpen',async event=>{
  try {
    await notificationNavigation.open(event, {
      cancelConnection: () => { connectionAttempt.cancel(); releaseConnectionLoading(); lock(false); },
      disconnect: () => Remote.disconnect(),
      showConnections: async () => { await initialized; history.pushState(null,'','#/'); await render(); },
      connect: target => connect(target.id, target.sessionId),
    });
  } catch(error) {status(String(error),true)}
});

export interface LoginResult {linked?:boolean;pending?:boolean}
export type LoginPhase='idle'|'starting'|'waiting'|'retrying'|'linked'|'error';
export interface LoginState {phase:LoginPhase;message:string}
type AccountRequest=(action:'login'|'poll')=>Promise<LoginResult>;

/** Keep authorization polling alive when the user leaves the account page or returns from the browser. */
export class AccountLogin {
  state:LoginState={phase:'idle',message:''};
  private generation=0;
  private timer:ReturnType<typeof setTimeout>|undefined;
  private polling=false;
  private listeners=new Set<(state:LoginState)=>void>();
  private request:AccountRequest;
  private retryDelay:number;
  constructor(request:AccountRequest,retryDelay=2000) {this.request=request;this.retryDelay=retryDelay}
  get active() {return ['starting','waiting','retrying'].includes(this.state.phase)}
  subscribe(listener:(state:LoginState)=>void) {this.listeners.add(listener);return ()=>{this.listeners.delete(listener)}}
  private update(phase:LoginPhase,message='') {this.state={phase,message};for(const listener of this.listeners)listener(this.state)}
  reset() {this.generation++;clearTimeout(this.timer);this.timer=undefined;this.polling=false;this.update('idle')}
  async start() {
    if(this.active)return;
    this.reset();const generation=this.generation;this.update('starting','正在打开登录窗口…');
    try {
      await this.request('login');
      if(generation!==this.generation)return;
      this.update('waiting','请在登录窗口中完成登录，然后返回 App。');
      await this.check();
    }catch(error){if(generation===this.generation)this.update('error',error instanceof Error?error.message:String(error))}
  }
  resume() {
    if(!this.active)this.update('waiting','正在检查登录结果…');
    return this.check();
  }
  async check() {
    if(!this.active || this.state.phase==='starting' || this.polling)return;
    clearTimeout(this.timer);this.timer=undefined;this.polling=true;
    const generation=this.generation;
    try {
      const result=await this.request('poll');
      if(generation!==this.generation)return;
      if(result.linked){this.update('linked','登录成功。');return;}
      this.update('waiting','等待登录确认。完成后将自动更新账号和设备列表。');
    }catch(error){
      if(generation!==this.generation)return;
      const code=(error as {code?:string})?.code;
      if(code==='ACCOUNT_LOGIN_EXPIRED' || code==='ACCOUNT_AUTH_REQUIRED') {
        this.update('error','登录请求已失效，请重新登录。');return;
      }
      this.update('retrying','暂时无法连接账号服务，正在重试。无需重新登录。');
    }finally{
      if(generation===this.generation){
        this.polling=false;
        if(this.active)this.timer=setTimeout(()=>void this.check(),this.retryDelay);
      }
    }
  }
}

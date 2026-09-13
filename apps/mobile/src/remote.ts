import { registerPlugin, type PluginListenerHandle } from '@capacitor/core';
export interface Connection {
  id?: string; name: string; mode: 'ssh' | 'url'; url?: string;
  host?: string; port?: number; username?: string; auth?: 'password' | 'key';
  password?: string; privateKey?: string; passphrase?: string;
  service?: 'auto' | 'manual'; remotePort?: number; prepare?: boolean;
  webPassword?: string; hasSecret?: boolean; hasWebPassword?: boolean;
}
export type ScanResult = {cancelled:true} | {url:string;name:string};
export interface AccountState {linked?:boolean;pending?:boolean;account?:{id:string;displayName:string};devices?:{id:string;name:string;online:boolean;sharing:boolean;access:{id:string;scope:string;name:string;ready:boolean}[]}[]}
export interface PushStatus {configured:boolean;enabled:boolean;permission:string;connections:string[];error?:string}
interface RemotePlugin {
  notifications(options:{action:'status'|'enable'|'disable'|'test';id?:string}):Promise<PushStatus>;
  account(options:{action:"status"|"login"|"poll"|"logout"|"devices"|"open";deviceId?:string;grantId?:string;sessionId?:string}):Promise<AccountState>;
  scanURL(): Promise<ScanResult>;
  list(): Promise<{connections: Connection[]}>;
  save(options: {connection: Connection; copyFromId?: string}): Promise<{connection: Connection}>;
  remove(options: {id: string}): Promise<void>;
  connect(options: {id: string; sessionId?: string}): Promise<{id: string}>;
  disconnect(): Promise<void>;
  status(): Promise<{connected: boolean; id?: string}>;
  addListener(event: 'state', listener: (event: {phase: string}) => void): Promise<PluginListenerHandle>;
  addListener(event: 'notificationOpen', listener: (event: {id: string; sessionId?: string}) => void): Promise<PluginListenerHandle>;
}
export const Remote = registerPlugin<RemotePlugin>('VelaRemote');

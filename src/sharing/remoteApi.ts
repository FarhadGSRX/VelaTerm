import { invoke } from "../ipc/transport";
import { getLocale } from "../i18n";
import { sharingText } from "./copy";

/** One remote range enabled on a device, as returned by the account service. */
export interface RemoteGrant {
  id: string;
  scope: string;
  name: string;
  deviceName: string;
  publicKey: string;
  ready: boolean;
  expiresAt: string;
}

/** One account device with the remote ranges it has enabled. */
export interface RemoteDevice {
  online: boolean;
  sharing: boolean;
  id: string;
  name: string;
  publicKey: string;
  access: RemoteGrant[];
}

export const remoteDevices = () => invoke<RemoteDevice[]>("public_account_remote_devices");
export const remoteText = (key: string) => sharingText(key, getLocale());

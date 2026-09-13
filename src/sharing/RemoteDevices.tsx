import { useEffect, useState } from "react";
import { invoke } from "../ipc/transport";
import { remoteDevices, type RemoteDevice } from "./remoteApi";
import { SharingLink } from "./navigation";
import { remoteText as t } from "./remoteApi";
import "./remote-access.css";
export function RemoteDevices({onClose}: {onClose: () => void}) {
  const [devices,setDevices] = useState<RemoteDevice[]>([]);
  const [linked,setLinked] = useState(false);
  const [ready,setReady] = useState(false);
  const [error,setError] = useState(false);
  const [busy,setBusy] = useState<string>();
  useEffect(() => {
    let stopped = false;
    let timer: ReturnType<typeof setTimeout>;
    const refresh = async () => {
      try {
        const status = await invoke<{linked:boolean}>("public_account_status");
        const list = status.linked ? await remoteDevices() : [];
        if (!stopped) {setLinked(status.linked);setDevices(list);setError(false);}
      } catch {if (!stopped) {setDevices([]);setError(true);}}
      finally {if (!stopped) {setReady(true);timer=setTimeout(() => void refresh(),5000);}}
    };
    void refresh();
    return () => {stopped=true;clearTimeout(timer);};
  }, []);
  return <section className="remote-devices">
    {!ready && <p role="status">{t("Loading…")}</p>}
    {error && <p role="alert">{t("Service unavailable. Please try again.")}</p>}
    {ready && !linked && !error && <><p>{t("remote.login")}</p><SharingLink values={{publicAccount:"1",connect:null}} onClick={onClose}>{t("Sign in to VelaTerm")}</SharingLink></>}
    {ready && linked && !error && !devices.length && <p>{t("remote.no_clients")}</p>}
    {devices.map(d => <div className="remote-device" key={d.id}><div><strong>{d.name}</strong><small className="remote-device-status">{t(d.online ? "remote.online" : "remote.offline")}</small><small>{d.access.length ? d.access.map(scope => scope.scope === "machine" ? t("remote.workspace") : scope.name === scope.scope ? t(`remote.${scope.scope}`) : scope.name).join(" · ") : t("remote.empty")}</small></div>{d.online === true && d.sharing === true && <button disabled={!!busy} onClick={() => {
      setBusy(d.id);setError(false);
      void invoke("open_account_remote_window", {deviceId:d.id}).then(onClose).catch(() => setError(true)).finally(() => setBusy(undefined));
    }}>{busy === d.id ? t("Loading…") : t("remote.view") + " →"}</button>}</div>)}
  </section>;
}

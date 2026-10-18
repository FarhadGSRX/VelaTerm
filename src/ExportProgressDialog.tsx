import { useSyncExternalStore } from "react";
import { Backdrop } from "./components/Backdrop";
import { useT } from "./i18n";
import { exportProgressSnapshot, subscribeExportProgress } from "./exportSession";

/** Closing this dialog cancels only file generation; the agent process is independent. */
export function ExportProgressDialog() {
  const t=useT();const jobs=useSyncExternalStore(subscribeExportProgress,exportProgressSnapshot);
  if (!jobs.length) return null;
  const cancel=()=>{for (const job of jobs) job.controller.abort();};
  return <Backdrop onClose={cancel}>
    <div className="vlx-panel" role="dialog" aria-modal="true" aria-label={t("export.preparing")} onKeyDown={event=>{
      if (event.key==="Escape") {event.stopPropagation();cancel();}
    }} style={{padding:20,width:360,maxWidth:"90vw",borderRadius:10,background:"var(--bg-elevated)",border:"1px solid var(--border-strong)",boxShadow:"var(--shadow)"}}>
      <div role="status">{t("export.preparing")}</div>
      {jobs.map((job,index)=><div key={index} style={{marginTop:10,overflowWrap:"anywhere"}}>{job.name}</div>)}
      <button className="vlx-btn" autoFocus style={{marginTop:16}} onClick={cancel}>{t("common.cancel")}</button>
    </div>
  </Backdrop>;
}

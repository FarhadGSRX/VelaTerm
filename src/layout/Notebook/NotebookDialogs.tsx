import { useEffect, useRef, useState } from "react";
import { useT } from "../../i18n";
import Select from "../../components/Select";
import { notebookCreate, notebookDroppedFiles, notebookMove, notebookTrash, notebookUnregister, type NotebookOverview, type NotebookTree, type ImportResult } from "../../ipc/notebook";
import { memoryGet } from "../../ipc/memory";
import { showVaultAction } from "./VaultActions";
import { MemoryLink, memoryNavigate, memoryUrl } from "../Memory/navigation";
import { NotebookError, notebookError, notebookRoute, openVaultFile, sizeLabel } from "./shared";
import { NotebookImportCancelled, cancelNotebookImport, startNotebookImport, useNotebookImports } from "./importManager";

export function NotebookDialog({action,tree,overview,onChanged}:{action:string;tree:NotebookTree;overview:NotebookOverview;onChanged:()=>void}) {
  const t=useT();const params=new URLSearchParams(location.search);const id=tree.vault.id;const current=params.get("memoryPath")??"";
  const [name,setName]=useState(params.get("memoryName")??(action==="move"?current:""));
  const [parent,setParent]=useState(params.get("memoryFolder")??current.slice(0,Math.max(0,current.lastIndexOf('/'))));
  const [files,setFiles]=useState<File[]>([]);const [result,setResult]=useState<ImportResult|null>(null);
  const [busy,setBusy]=useState(false);const [error,setError]=useState("");
  const running=useNotebookImports().get(id);const fileInput=useRef<HTMLInputElement>(null);const folderInput=useRef<HTMLInputElement>(null);
  const title=action==="new"?"nb.newNote":action==="folder"?"nb.newFolder":action==="move"?"nb.move":action==="delete"?"common.delete":action==="close"?"nb.closeVault":"nb.import";
  const finish=()=>{onChanged();memoryNavigate(memoryUrl(notebookRoute(id)),false,true);};
  const submit=async()=>{
    setBusy(true);setError("");
    try {
      if(action==="new"||action==="folder") {
        const copyPath=params.get("memoryCopyPath");const copied=copyPath?sessionStorage.getItem(`nb-copy:${id}:${copyPath}`):undefined;
        const created=await notebookCreate(id,parent,name,action==="folder"?"folder":"note",copied??undefined);
        if(copyPath)sessionStorage.removeItem(`nb-copy:${id}:${copyPath}`);
        onChanged();
        if(action==="folder")memoryNavigate(memoryUrl(notebookRoute(id),{memoryPath:null,memoryFolder:created.path,memoryName:null,memoryCopyPath:null}),false,true);
        else openVaultFile(created.absolutePath);
      }else if(action==="move") {await notebookMove(id,current,name);finish();}
      else if(action==="delete") {await notebookTrash(id,current);finish();}
      else if(action==="close") {await notebookUnregister(id);onChanged();window.dispatchEvent(new Event("notebook:vaultsChanged"));memoryNavigate(memoryUrl("notebooks",{memoryPath:null,memoryFolder:null,memoryFilter:null,memoryQuery:null,memoryTag:null}),false,true);}
      else if(action==="import") {setResult(await startNotebookImport(id,parent,files,overview.limits.chunkBytes));onChanged();}
    }catch(e){if(!(e instanceof NotebookImportCancelled))setError(notebookError(e));}
    finally{setBusy(false);}
  };
  return <form className="nb-dialog" onSubmit={e=>{e.preventDefault();void submit();}}>
    <header><h2>{t(title)}</h2>{busy?<button type="button" className="btn" onClick={()=>cancelNotebookImport(id)}>{t("common.cancel")}</button>:<MemoryLink route={notebookRoute(id)} values={{memoryPath:null}}>{t("common.cancel")}</MemoryLink>}</header>
    {(action==="new"||action==="folder"||action==="move")&&<label>{t(action==="move"?"nb.destination":"nb.name")}<input className="input" autoFocus required value={name} onChange={e=>setName(e.target.value)}/></label>}
    {(action==="new"||action==="folder"||action==="import")&&<label>{t("nb.folder")}<Select width="100%" value={parent} ariaLabel={t("nb.folder")} onChange={setParent} options={[{ value: "", label: tree.vault.name }, ...tree.nodes.filter(n=>n.kind==="folder").map(n=>({ value: n.path, label: n.path }))]} /></label>}
    {action==="move"&&<p>{t("nb.moveHint")}</p>}
    {action==="delete"&&<p>{t("nb.trashHint")}<strong className="nb-block">{current}</strong></p>}
    {action==="close"&&<p>{t("nb.closeHint")}</p>}
    {action==="import"&&<><p>{t("nb.importHint")}</p><div className="nb-import-drop" onDragOver={e=>e.preventDefault()} onDrop={e=>{e.preventDefault();void notebookDroppedFiles(e.dataTransfer).then(setFiles).catch(error=>setError(notebookError(error)));}}><div className="nb-inline"><button type="button" className="btn" disabled={busy} onClick={()=>fileInput.current?.click()}>{t("nb.importFiles")}</button><button type="button" className="btn" disabled={busy} onClick={()=>folderInput.current?.click()}>{t("nb.importFolder")}</button></div><span>{files.length} {t("nb.files")} · {sizeLabel(files.reduce((n,f)=>n+f.size,0))}</span></div><input hidden ref={fileInput} type="file" multiple onChange={e=>setFiles(Array.from(e.target.files??[]))}/><input hidden ref={folderInput} type="file" multiple {...{webkitdirectory:""}} onChange={e=>setFiles(Array.from(e.target.files??[]))}/><div className="nb-import-manifest">{files.slice(0,30).map((f,i)=><div key={i}><span>{f.webkitRelativePath||f.name}</span><small>{sizeLabel(f.size)}</small></div>)}</div>{(busy||running)&&<div className="nb-import-live" role="status">{running?<progress max={1} value={running.bytes?running.sent/running.bytes:1}/>:<progress/>}<span>{running?`${t("nb.importProgress",String(running.done),String(running.files))} · ${running.current}`:t("common.loading")}</span>{running&&<MemoryLink route={notebookRoute(id,"imports")}>{t("nb.imports")}</MemoryLink>}</div>} {result&&<div className="nb-banner" role="status">{t("nb.imported")}: {result.imported} · {t("nb.skipped")}: {result.skipped}<button type="button" className="btn" onClick={()=>{const index=result.paths.findIndex(p=>/\.md$/i.test(p));const path=result.absolutePaths[index>=0?index:0];if(path)openVaultFile(path);}}>{t("common.open")}</button></div>}</>}
    {error&&<NotebookError error={error}/>}
    {!result&&<footer><button className="btn btn-primary" disabled={busy||(action==="import"&&(!files.length||!!running))}>{t(busy?"common.loading":"common.confirm")}</button></footer>}
  </form>;
}

export function CopyKnowledgeDialog({overview,onChanged}:{overview:NotebookOverview;onChanged:()=>void}) {
  const t=useT();const entryId=new URLSearchParams(location.search).get("memoryEntry")??"";
  const [vaultId,setVaultId]=useState(overview.vaults[0]?.id??"");const [name,setName]=useState("");const [content,setContent]=useState("");const [error,setError]=useState("");const [busy,setBusy]=useState(false);
  useEffect(()=>{let alive=true;memoryGet(entryId).then(({entry})=>{if(alive){setName(entry.title);setContent(`# ${entry.title}\n\n${entry.summary}\n\n${entry.content}\n`);}}).catch(e=>{if(alive)setError(notebookError(e));});return()=>{alive=false;};},[entryId]);
  return <form className="nb-dialog" onSubmit={e=>{e.preventDefault();setBusy(true);void notebookCreate(vaultId,"",name,"note",content).then(note=>{onChanged();openVaultFile(note.absolutePath);}).catch(e=>setError(notebookError(e))).finally(()=>setBusy(false));}}><header><h2>{t("nb.copyTo")}</h2><MemoryLink route={`entry/${entryId}`}>{t("common.cancel")}</MemoryLink></header>{!overview.vaults.length?<><p>{t("nb.empty")}</p><button type="button" className="btn" onClick={()=>showVaultAction("open")}>{t("nb.openVault")}</button></>:<><label>{t("nb.vaults")}<Select width="100%" value={vaultId} ariaLabel={t("nb.vaults")} onChange={setVaultId} options={overview.vaults.map(v=>({ value: v.id, label: v.name }))} /></label><label>{t("nb.name")}<input className="input" required value={name} onChange={e=>setName(e.target.value)}/></label><footer><button className="btn btn-primary" disabled={busy||!content}>{t("common.save")}</button></footer></>}{error&&<NotebookError error={error}/>}</form>;
}

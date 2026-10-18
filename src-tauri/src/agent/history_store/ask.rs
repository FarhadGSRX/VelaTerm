//! Bounded reference context delivery; the existing full/summary policy stays explicit.

use std::fs::{File,OpenOptions};
use std::io::{Read,Seek,SeekFrom,Write};
use std::path::{Path,PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool,Ordering};
use std::time::{Duration,Instant};
use sha2::{Digest,Sha256};
use crate::{agent::{headless,server},db::repo,host::AppCtx,models::Session};
use super::prose::Prose;

pub fn answer(app:&AppCtx,request:&server::ReferRequest,session:&Session,prose:Prose,cancel:Arc<AtomicBool>) -> Result<server::Answered,String> {
    if server::ask_disabled(app) {return Err("session question answering is turned off in settings".into());}
    let question=request.ask.as_deref().map(str::trim).filter(|text|!text.is_empty()).ok_or("A reference question is required")?;
    let started=Instant::now();let timeout=Duration::from_secs(request.timeout.map(u64::from).unwrap_or(server::DEFAULT_ASK_TIMEOUT_SECS));
    let cwd=session.cwd.as_deref().map(Path::new).filter(|path|path.is_dir());
    let caller=repo::get_session_kind(&app.db().conn.lock().unwrap(),&request.session_id).ok().flatten();
    let want=request.with.as_deref().map(str::trim).filter(|text|!text.is_empty()).map(crate::models::SessionKind::from_db);
    let total=prose.total;
    let (result,kind,pre_summary,search_matches)=if let Some(selection)=server::refer_summary_selection(app)? {
        let summary_started=Instant::now();
        let summary=summarize(app,question,&prose,&selection,cwd,started,timeout,&cancel)?;
        if cancel.load(Ordering::Relaxed) {return Err("read_canceled".into());}
        let excerpts=server::relevant_original_excerpts(app,&session.id,question)?;let count=excerpts.len();
        let prompt=server::build_compressed_ask_prompt(question,&summary,&excerpts);
        let picked=headless::pick(app,want,caller,prompt.len()).ok_or("no installed agent can answer this session question")?;
        let result=server::run_reference_agent_with_cancel(&picked,&prompt,None,cwd,server::remaining_timeout(started,timeout)?,"answer_session_question",Some(&cancel));
        (result,picked.kind,Some(server::PreSummaryRun {selection,elapsed_ms:summary_started.elapsed().as_millis()}),count)
    } else {
        let prefix=server::build_ask_prompt(question,&[]);
        let mut counted=Count::default();counted.write_all(prefix.as_bytes()).map_err(|e|e.to_string())?;
        prose.write_model_context(&cancel,&mut counted)?;
        let picked=headless::pick(app,want,caller,usize::try_from(counted.bytes).unwrap_or(usize::MAX)).ok_or("no installed agent can accept the complete session context")?;
        let source_cancel=cancel.clone();let kind=picked.kind;
        crate::diagnostics::record("INFO","ai_request",serde_json::json!({"event":"ai_request","step":"answer_session_question","method":"AI",
            "agent":kind.as_str(),"contextMode":"full","originalChars":counted.chars,"sentChars":counted.chars,"inputBytes":counted.bytes,
            "sha256":format!("{:x}",counted.hash.finalize()),"truncated":false,"preview":"[session reference content redacted]","status":"started"}));
        let source:headless::PromptSource=Box::new(move |mut writer| {
            writer.write_all(prefix.as_bytes()).map_err(|e|e.to_string())?;
            prose.write_model_context(&source_cancel,&mut writer)
        });
        let result=headless::run_selected_stream(&picked,source,None,cwd,server::remaining_timeout(started,timeout)?,&cancel).map_err(|e|e.to_string());
        crate::diagnostics::record(if result.is_ok() {"INFO"} else {"ERROR"},"ai_response",serde_json::json!({"event":"ai_response",
            "step":"answer_session_question","method":"AI","agent":kind.as_str(),"status":if result.is_ok() {"completed"} else {"failed"},
            "durationMs":started.elapsed().as_millis() as u64,"responseBytes":result.as_ref().map(|value|value.len()).unwrap_or(0)}));
        (result,kind,None,0)
    };
    let answer=result?;let elapsed_ms=started.elapsed().as_millis();
    crate::diagnostics::record("INFO","refer_answer",serde_json::json!({"event":"refer_answer","step":"answer","method":"AI","inputCount":total,
        "outputCount":1,"contextMode":if pre_summary.is_some() {"summary"} else {"full"},"searchMatches":search_matches,"durationMs":elapsed_ms as u64,"status":"success"}));
    Ok(server::Answered {answer,kind,elapsed_ms,pre_summary,search_matches})
}

#[derive(Default)]
struct Count {bytes:u64,chars:u64,hash:Sha256}
impl Write for Count {
    fn write(&mut self,bytes:&[u8]) -> std::io::Result<usize> {
        let text=std::str::from_utf8(bytes).map_err(std::io::Error::other)?;
        self.bytes=self.bytes.checked_add(bytes.len() as u64).ok_or_else(||std::io::Error::other("Context size overflow"))?;
        self.chars+=text.chars().count() as u64;self.hash.update(bytes);Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {Ok(())}
}

/// Every callback receives a complete UTF-8 chunk; only one chunk and one source block are retained.
struct Chunks<'a> {pending:String,limit:usize,sink:&'a mut dyn FnMut(&str)->Result<(),String>}
impl Chunks<'_> {
    fn finish(&mut self) -> Result<(),String> {if !self.pending.is_empty() {(self.sink)(&self.pending)?;self.pending.clear();}Ok(())}
}
impl Write for Chunks<'_> {
    fn write(&mut self,bytes:&[u8]) -> std::io::Result<usize> {
        let mut text=std::str::from_utf8(bytes).map_err(std::io::Error::other)?;
        while !text.is_empty() {
            let mut end=text.len().min(self.limit-self.pending.len());
            while !text.is_char_boundary(end) {end-=1;}
            if end==0 {self.finish().map_err(std::io::Error::other)?;continue;}
            self.pending.push_str(&text[..end]);text=&text[end..];
            if self.pending.len()==self.limit {self.finish().map_err(std::io::Error::other)?;}
        }Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {Ok(())}
}

/// Generated summaries are task scratch data, never a second copy of native conversation bodies.
struct Scratch {file:Option<File>,path:PathBuf,bytes:u64,store:Arc<super::store::Store>}
impl Scratch {
    fn new(app:&AppCtx) -> Result<Self,String> {
        let root=app.data_dir()?.join("chat-index").join("tasks");std::fs::create_dir_all(&root).map_err(|_|"storage_pressure: summary scratch storage is unavailable")?;
        let path=root.join(format!("{}.summary",uuid::Uuid::new_v4()));let mut options=OpenOptions::new();options.read(true).write(true).create_new(true);
        #[cfg(unix)] {use std::os::unix::fs::OpenOptionsExt;options.mode(0o600);}
        let store=super::store::Store::at(&app.data_dir()?)?;
        let file=options.open(&path).map_err(|_|"storage_pressure: summary scratch storage is unavailable")?;
        if let Err(error)=store.reserve_scratch(&path,0) {drop(file);let _=std::fs::remove_file(&path);return Err(error);}
        Ok(Self {file:Some(file),path,bytes:0,store})
    }
    fn append(&mut self,number:usize,text:&str) -> Result<(),String> {
        let header=format!("{}[summary part {number}]\n",if number>1 {"\n\n"} else {""});
        let next=self.bytes.checked_add((header.len()+text.len()) as u64).ok_or("storage_pressure: summary scratch size overflow")?;
        if next>64*1024*1024 {return Err("storage_pressure: generated summaries exceed their 64 MiB task budget".into());}
        self.store.reserve_scratch(&self.path,next)?;
        let file=self.file.as_mut().unwrap();file.write_all(header.as_bytes()).and_then(|_|file.write_all(text.as_bytes())).map_err(|_|"storage_pressure: generated summaries could not be saved")?;self.bytes=next;Ok(())
    }
    fn walk(&mut self,sink:&mut dyn FnMut(&str)->Result<(),String>) -> Result<(),String> {
        let file=self.file.as_mut().unwrap();file.seek(SeekFrom::Start(0)).map_err(|e|e.to_string())?;
        let mut block=[0;32*1024];let mut carry=Vec::with_capacity(32*1024+4);
        loop {
            let count=file.read(&mut block).map_err(|e|e.to_string())?;
            if count==0 {if !carry.is_empty() {return Err("Generated summary has an incomplete UTF-8 suffix".into());}return Ok(());}
            carry.extend_from_slice(&block[..count]);
            let valid=match std::str::from_utf8(&carry) {Ok(text)=>text.len(),Err(error) if error.error_len().is_none()=>error.valid_up_to(),Err(_)=>return Err("Generated summary has invalid UTF-8".into())};
            sink(std::str::from_utf8(&carry[..valid]).map_err(|e|e.to_string())?)?;carry.drain(..valid);
        }
    }
}
impl Drop for Scratch {fn drop(&mut self) {
    drop(self.file.take());
    match std::fs::remove_file(&self.path) {Ok(())=>self.store.release_scratch(&self.path),Err(error) if error.kind()==std::io::ErrorKind::NotFound=>self.store.release_scratch(&self.path),Err(_)=>{}}
}}

fn summarize(app:&AppCtx,question:&str,prose:&Prose,selection:&crate::agent::launch_options::AgentSelection,cwd:Option<&Path>,started:Instant,timeout:Duration,cancel:&AtomicBool) -> Result<String,String> {
    let picked=headless::pick(app,Some(selection.agent),None,0).ok_or("The configured summarizer is not installed or configured")?;
    let limit=server::SUMMARY_CHUNK_BYTES;let mut total=0;
    {
        let mut count=|_:&str| {total+=1;Ok(())};let mut chunks=Chunks {pending:String::with_capacity(limit),limit,sink:&mut count};
        prose.write_model_context(cancel,&mut chunks)?;chunks.finish()?;
    }
    let mut summaries=Scratch::new(app)?;let mut part=0;
    {
        let mut summarize=|text:&str| {
            if cancel.load(Ordering::Relaxed) {return Err("read_canceled".into());}part+=1;
            let prompt=server::build_summary_prompt(question,text,part,total.max(1),false);
            let answer=server::run_reference_agent_with_cancel(&picked,&prompt,Some(selection),cwd,server::remaining_timeout(started,timeout)?,"compress_session_transcript",Some(cancel))?;
            summaries.append(part,&server::limit_chars(&answer,server::SUMMARY_PART_CHARS))
        };
        if total==0 {summarize("")?;}
        let mut chunks=Chunks {pending:String::with_capacity(limit),limit,sink:&mut summarize};
        prose.write_model_context(cancel,&mut chunks)?;chunks.finish()?;
    }
    for _ in 0..8 {
        if summaries.bytes<=limit as u64 {let mut combined=String::with_capacity(summaries.bytes as usize);summaries.walk(&mut |text|{combined.push_str(text);Ok(())})?;return Ok(combined);}
        let mut total=0;
        {
            let mut count=|_:&str|{total+=1;Ok(())};let mut chunks=Chunks {pending:String::with_capacity(limit),limit,sink:&mut count};
            summaries.walk(&mut |text|chunks.write_all(text.as_bytes()).map_err(|e|e.to_string()))?;chunks.finish()?;
        }
        let mut reduced=Scratch::new(app)?;let mut part=0;
        {
            let mut reduce=|text:&str| {
                if cancel.load(Ordering::Relaxed) {return Err("read_canceled".into());}part+=1;
                let prompt=server::build_summary_prompt(question,text,part,total,true);
                let answer=server::run_reference_agent_with_cancel(&picked,&prompt,Some(selection),cwd,server::remaining_timeout(started,timeout)?,"reduce_session_summary",Some(cancel))?;
                reduced.append(part,&server::limit_chars(&answer,server::SUMMARY_PART_CHARS))
            };
            let mut chunks=Chunks {pending:String::with_capacity(limit),limit,sink:&mut reduce};
            summaries.walk(&mut |text|chunks.write_all(text.as_bytes()).map_err(|e|e.to_string()))?;chunks.finish()?;
        }
        summaries=reduced;
    }Err("the configured summarizer did not reduce the transcript enough".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    #[test]
    fn fake_model_receives_full_original_context_or_the_explicit_summary_policy() {
        use std::os::unix::fs::PermissionsExt;
        for summarize in [false,true] {
            let temp=super::super::tests::Temp::new();let captured=temp.0.join("model-input");let binary=temp.0.join("fake-claude");
            std::fs::write(&binary,format!("#!/bin/sh\ncat >> '{}'\nprintf 'summary decision'\n",captured.display())).unwrap();
            std::fs::set_permissions(&binary,std::fs::Permissions::from_mode(0o700)).unwrap();
            let db=crate::db::Db::open(&temp.0.join("main.sqlite")).unwrap();let app=AppCtx::Headless(Arc::new(crate::host::HeadlessHost::new(temp.0.clone(),db)));
            let session={
                let conn=app.db().conn.lock().unwrap();let project=repo::create_virtual_project(&conn,"reference test").unwrap();
                let session=repo::create_session(&conn,&project.id,None,"reference",crate::models::SessionKind::Codex,None,None,None,None,None).unwrap();
                repo::set_app_settings(&conn,&std::collections::HashMap::from([("vlx-settings".into(),serde_json::json!({
                    "agentDefaults":{"claude":{"path":binary}},"referSummary":{"enabled":summarize,"agent":"claude","model":"test-model","effort":"low"}
                }).to_string())])).unwrap();session
            };
            let text="original 雪😀 decision\n".repeat(40_000);let raw=serde_json::json!({"type":"response_item","timestamp":"t1","payload":{
                "type":"message","role":"user","content":[{"type":"input_text","text":text}]}}).to_string()+"\n";
            let branch=super::super::tests::indexed(&temp,&raw);let original=branch.sources[0].segment.path.clone();
            let cancel=Arc::new(AtomicBool::new(false));let prose=Prose::new(branch,&cancel).unwrap();
            let request=serde_json::from_value(serde_json::json!({"sessionId":session.id,"target":session.id,"ask":"Which decision?","with":"claude","timeout":30})).unwrap();
            let result=answer(&app,&request,&session,prose,cancel).unwrap();assert_eq!(result.answer,"summary decision");
            let input=std::fs::read_to_string(captured).unwrap();
            if summarize {
                assert!(result.pre_summary.is_some());assert!(input.contains("COMPRESSED SESSION SUMMARY:"));
                let full=format!("\n[#0 user] t1\n{}\n",text.trim());let mut parts=Vec::new();
                let mut receive=|part:&str|{parts.push(part.to_owned());Ok(())};
                let mut chunks=Chunks {pending:String::new(),limit:server::SUMMARY_CHUNK_BYTES,sink:&mut receive};chunks.write_all(full.as_bytes()).unwrap();chunks.finish().unwrap();
                let mut expected=String::new();let mut summaries=String::new();
                for (index,part) in parts.iter().enumerate() {
                    expected.push_str(&server::build_summary_prompt("Which decision?",part,index+1,parts.len(),false));
                    if index>0 {summaries.push_str("\n\n");}summaries.push_str(&format!("[summary part {}]\nsummary decision",index+1));
                }
                expected.push_str(&server::build_compressed_ask_prompt("Which decision?",&summaries,&[]));assert_eq!(input,expected);
                assert!(!std::fs::read_dir(temp.0.join("chat-index/tasks")).unwrap().any(|entry|entry.unwrap().path().is_file()));
            } else {
                assert!(result.pre_summary.is_none());assert!(!input.contains("COMPRESSED SESSION SUMMARY:"));
                assert_eq!(input,format!("{}\n[#0 user] t1\n{}\n",server::build_ask_prompt("Which decision?",&[]),text.trim()));
            }
            assert_eq!(std::fs::read_to_string(original).unwrap(),raw);drop(app);
        }
    }

    #[test]
    fn streamed_summary_chunks_preserve_utf8_and_the_original_chunk_boundaries() {
        let text="abc雪😀 xyz\n".repeat(10_000);let limit=56*1024;let mut result=Vec::new();
        let mut receive=|chunk:&str|{assert!(chunk.len()<=limit);result.push(chunk.to_owned());Ok(())};
        let mut chunks=Chunks {pending:String::new(),limit,sink:&mut receive};
        for input in text.split_inclusive('\n') {chunks.write_all(input.as_bytes()).unwrap();}chunks.finish().unwrap();
        assert_eq!(result.concat(),text);
        let mut expected=Vec::new();let mut remaining=text.as_str();
        while !remaining.is_empty() {let mut end=remaining.len().min(limit);while !remaining.is_char_boundary(end) {end-=1;}expected.push(remaining[..end].to_owned());remaining=&remaining[end..];}
        assert_eq!(result,expected);
    }
}

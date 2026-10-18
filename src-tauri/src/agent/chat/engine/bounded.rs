//! The bounded protocol receive boundary and lossless approval responses.

use super::*;
use crate::agent::history_store::{live,projection::{Field,Record}};

#[derive(Clone)]
pub(super) struct Original {frame:live::Frame,field:Field}
impl Original {
    pub(super) fn is_partial(&self) -> bool {self.field.truncated}
}


pub(super) struct Outgoing {journal:live::Journal,archive:Arc<live::Archive>,latest:Option<live::Frame>,native:Option<String>,
    publisher:mpsc::SyncSender<(live::Frame,String)>,proposal_time:Option<(String,u64)>,pending_markers:std::collections::VecDeque<(String,Value)>}
fn publish_outgoing(outgoing:&Outgoing,session:&str,proc:&Arc<ChatProcess>) -> Result<(),String> {
    if let (Some(frame),Some(native))=(&outgoing.latest,&outgoing.native) {
        outgoing.archive.bind_outgoing(frame,proc.kind,session,proc.started_at,native)?;
        let _=outgoing.publisher.try_send((frame.clone(),native.clone()));
    }Ok(())
}
fn outgoing<'a>(slot:&'a mut Option<Outgoing>,app:&AppCtx,session:&str,proc:&Arc<ChatProcess>) -> Result<&'a mut Outgoing,String> {
    if slot.is_none() {
        let archive=live::Archive::at(&app.data_dir()?)?;
        *slot=Some(Outgoing {journal:live::Journal::new(archive.clone(),session,proc.started_at)?,
            publisher:publisher(app.clone(),session.to_owned(),proc.clone(),archive.clone(),true),archive,latest:None,
            native:proc.agent_session_id.lock().unwrap().clone(),proposal_time:None,pending_markers:std::collections::VecDeque::new()});
    }
    let outgoing=slot.as_mut().unwrap();
    while let Some((key,value))=outgoing.pending_markers.front() {
        outgoing.latest=Some(outgoing.journal.append(key,value,&proc.released)?);outgoing.pending_markers.pop_front();
    }
    let native=proc.agent_session_id.lock().unwrap().clone();
    if outgoing.native.is_some() && outgoing.native!=native {
        publish_outgoing(outgoing,session,proc)?;
        outgoing.journal=live::Journal::new(outgoing.archive.clone(),session,proc.started_at)?;outgoing.latest=None;
    }
    outgoing.native=native;publish_outgoing(outgoing,session,proc)?;Ok(outgoing)
}
#[derive(serde::Serialize)]
struct Proposal<'a> {#[serde(rename="type")] kind:&'static str,id:&'a str,text:&'a str,images:&'a [ChatImage],process_epoch:u64,at:u64}
pub(super) fn proposal(app:&AppCtx,session:&str,proc:&Arc<ChatProcess>,id:&str,text:&str,images:&[ChatImage],dispatch:bool) -> Result<Option<live::Frame>,String> {
    if !proc.bounded_protocol {return Ok(None);}
    let kind=if dispatch {"dispatch"} else {"proposal"};
    let mut slot=proc.outgoing.lock().unwrap();let outgoing=outgoing(&mut slot,app,session,proc)?;
    let key=format!("{kind}:{id}");
    if outgoing.journal.pending_key().is_some_and(|pending|pending!=key) {return Err("storage_pressure: an earlier outgoing original must finish before another submission can be saved".into());}
    if outgoing.proposal_time.as_ref().is_none_or(|(previous,_)|previous!=&key) {outgoing.proposal_time=Some((key.clone(),now_ms()));}
    let value=Proposal {kind,id,text,images,process_epoch:proc.started_at,at:outgoing.proposal_time.as_ref().unwrap().1};
    outgoing.latest=Some(outgoing.journal.append(&key,&value,&proc.released)?);publish_outgoing(outgoing,session,proc)?;Ok(outgoing.latest.clone())
}
pub(super) fn marker(app:&AppCtx,session:&str,proc:&Arc<ChatProcess>,id:&str,state:&str) {
    if !proc.bounded_protocol {return;}
    let value=json!({"type":"delivery","id":id,"state":state,"processEpoch":proc.started_at});let key=format!("delivery:{state}:{id}");
    save_marker(app,session,proc,key,value);
}
pub(super) fn identity(app:&AppCtx,session:&str,proc:&Arc<ChatProcess>,id:&str,native:&str,turn:&str) {
    if !proc.bounded_protocol {return;}
    let value=json!({"type":"identity","id":id,"nativeMessageId":native,"turnId":turn,"processEpoch":proc.started_at});
    save_marker(app,session,proc,format!("identity:{id}"),value);
}
fn save_marker(app:&AppCtx,session:&str,proc:&Arc<ChatProcess>,key:String,value:Value) {
    let mut slot=proc.outgoing.lock().unwrap();
    let result=outgoing(&mut slot,app,session,proc).and_then(|outgoing| {
        outgoing.latest=Some(outgoing.journal.append(&key,&value,&proc.released)?);publish_outgoing(outgoing,session,proc)
    });
    if result.is_err() {if let Some(outgoing)=slot.as_mut() {
        // Another proposal is refused until these markers have been saved. One dispatch can add
        // only its dispatching and pipe-written markers, so failed delivery records stay bounded.
        if !outgoing.pending_markers.iter().any(|(pending,_)|pending==&key) {outgoing.pending_markers.push_back((key,value));}
    }}
    if let Err(error)=result {emit(app,session,json!({"type":"storagePressure","message":error}));}
}

pub(super) fn enabled(kind:SessionKind) -> bool {
    // Claude and Codex read their protocol through the bounded receive boundary by default; `0` turns
    // it off for every provider. Any other value also enables the providers still being validated.
    match std::env::var("VLX_CHAT_BOUNDED_PROTOCOL").ok().as_deref() {
        Some("0") => false,
        Some(_) => kind!=SessionKind::Opencode,
        None => matches!(kind,SessionKind::Claude|SessionKind::Codex),
    }
}

pub(super) fn receive(app:&AppCtx,session:&str,proc:&Arc<ChatProcess>,stdout:std::process::ChildStdout) -> Result<(),String> {
    let data_dir=app.data_dir()?;
    let archive=retry_archive(&data_dir,&proc.released,&mut |error| {
        emit(app,session,json!({"type":"storagePressure","message":error}));
        std::thread::park_timeout(Duration::from_millis(250));Ok(())
    })?;
    let publisher=publisher(app.clone(),session.to_owned(),proc.clone(),archive.clone(),false);
    let mut receiver=live::Receiver::new(stdout,archive.clone(),session,proc.started_at);
    let mut native=proc.agent_session_id.lock().unwrap().clone();
    let mut last_pressure=None;
    loop {
        let current=proc.agent_session_id.lock().unwrap().clone();
        if native.is_some() && current!=native {receiver.rotate()?;}
        native=current;
        let next=receiver.next(&proc.released,&mut |_|Ok(()),&mut |error| {
            if last_pressure.is_none_or(|last:std::time::Instant|last.elapsed()>=Duration::from_secs(1)) {
                emit(app,session,json!({"type":"storagePressure","message":error}));last_pressure=Some(std::time::Instant::now());
            }
            // The unread suffix stays in the pipe. No approval, restart or termination is inferred
            // from a storage failure; retry until the user frees space or explicitly releases the process.
            std::thread::park_timeout(Duration::from_millis(250));
            if proc.released.load(Ordering::Relaxed) {return Err("read_canceled".into());}
            Ok(())
        })?;
        let Some((frame,record))=next else {return Ok(());};
        let _callback=proc.callbacks.read().unwrap();
        if !proc.released.load(Ordering::Relaxed) && app.chat().owns_process(session,proc) {
            handle_record(app,session,proc,&frame,&record);
            if let Some(outgoing)=proc.outgoing.lock().unwrap().as_mut() {
                if outgoing.native.is_none() {outgoing.native=proc.agent_session_id.lock().unwrap().clone();
                    if let Err(error)=publish_outgoing(outgoing,session,proc) {emit(app,session,json!({"type":"storagePressure","message":error}));}
                }
            }
            if matches!(proc.kind,SessionKind::Claude|SessionKind::Codex) && frame.complete {
                let current=proc.agent_session_id.lock().unwrap().clone();
                if let Some(identity)=native.as_ref().or(current.as_ref()) {
                    // Persist the scope before queuing reconstructible index work. Notifications can
                    // coalesce across a fork without leaving an earlier original unowned after a crash.
                    loop {match archive.bind(&frame,proc.kind,session,proc.started_at,identity) {
                        Ok(())=>break,
                        Err(error) if error.starts_with("storage_pressure:")=>{
                            if proc.released.load(Ordering::Relaxed) {return Err("read_canceled".into());}
                            emit(app,session,json!({"type":"storagePressure","message":error}));std::thread::park_timeout(Duration::from_millis(250));
                        },
                        Err(error)=>return Err(error),
                    }}
                    // Coalesce bounded source notifications. Every original byte was already saved;
                    // the queued notification scans the current persisted prefix, including frames
                    // received after it. Control delivery never waits for the derived index writer.
                    let _=publisher.try_send((frame,identity.clone()));
                }
                if native.is_some() && current!=native {receiver.rotate()?;}
                native=current;
            }
        }
    }
}

fn publisher(app:AppCtx,session:String,proc:Arc<ChatProcess>,archive:Arc<live::Archive>,outgoing:bool) -> mpsc::SyncSender<(live::Frame,String)> {
    let (sender,receiver)=mpsc::sync_channel::<(live::Frame,String)>(1);
    let kind=proc.kind;let epoch=proc.started_at;let process=Arc::downgrade(&proc);
    std::thread::spawn(move|| {
        loop {
            let (mut frame,mut native)=match receiver.recv_timeout(Duration::from_millis(250)) {
                Ok(value)=>value,
                Err(mpsc::RecvTimeoutError::Disconnected)=>return,
                Err(mpsc::RecvTimeoutError::Timeout)=>{
                    let Some(proc)=process.upgrade() else {return;};
                    if !proc.alive.load(Ordering::Relaxed) || proc.released.load(Ordering::Relaxed) || app.chat().closing.load(Ordering::Relaxed) {return;}
                    continue;
                },
            };
            let mut reported=false;
            loop {
                let result=if outgoing {archive.publish_outgoing(&frame,kind,&session,epoch,&native)} else {archive.publish(&frame,kind,&session,epoch,&native)};
                match result {
                    Ok(())=>break,
                    Err(error)=> {
                        let Some(proc)=process.upgrade() else {return;};
                        if !reported && app.chat().owns_process(&session,&proc) {
                            let event=if error.starts_with("storage_pressure:") {"storagePressure"} else {"historyReadError"};
                            emit(&app,&session,json!({"type":event,"message":error}));reported=true;
                        }
                        if proc.released.load(Ordering::Relaxed) || app.chat().closing.load(Ordering::Relaxed) {return;}
                        match receiver.recv_timeout(Duration::from_millis(250)) {
                            Ok((next,id))=>{frame=next;native=id;},
                            Err(mpsc::RecvTimeoutError::Disconnected)=>return,
                            Err(mpsc::RecvTimeoutError::Timeout)=>{},
                        }
                    }
                }
            }
        }
    });sender
}

fn retry_archive(path:&std::path::Path,cancel:&AtomicBool,pressure:&mut impl FnMut(&str)->Result<(),String>) -> Result<Arc<live::Archive>,String> {
    loop {
        if cancel.load(Ordering::Relaxed) {return Err("read_canceled".into());}
        match live::Archive::at(path) {
            Ok(archive)=>return Ok(archive),
            Err(error) if error.starts_with("storage_pressure:")=>pressure(&error)?,
            Err(error)=>return Err(error),
        }
    }
}

fn handle_record(app:&AppCtx,session:&str,proc:&Arc<ChatProcess>,frame:&live::Frame,record:&Record) {
    if !record.complete || record.error.is_some() {
        let error=record.error.as_deref().unwrap_or("index_incomplete: protocol frame is not complete");
        emit(app,session,json!({"type":"error","message":error}));return;
    }
    if proc.kind==SessionKind::Claude && record.value["type"]=="control_request" && record.value.pointer("/request/subtype").and_then(Value::as_str)==Some("can_use_tool") {
        if let (Some(id),Some(field))=(record.value["request_id"].as_str(),record.fields.iter().rev().find(|field|field.path=="/request/input")) {
            proc.permission_originals.lock().unwrap().insert(id.into(),Original {frame:frame.clone(),field:field.clone()});
        }
    }
    // Existing semantic handlers receive only the bounded projection. Original arguments are held
    // separately and cannot accidentally be replaced by this short value when answering an approval.
    handle_line(app,session,proc,&record.value.to_string());
    runtime::bound(proc,record.fields.iter().any(|field|field.truncated));
}

pub(super) fn respond_original(proc:&ChatProcess,request:&str,original:&Original,updates:Option<&Value>) -> Result<(),String> {
    // Check the complete pinned proposal before writing the response envelope. A changed source must
    // leave the permission pending and must not send a partial approval as if it had succeeded.
    live::write_raw_field(&original.frame,&original.field,&mut std::io::sink(),&proc.released)?;
    let mut guard=proc.stdin.lock().unwrap();
    let stdin=guard.as_mut().ok_or("The agent is not accepting input")?;
    protocol::write_permission_original(stdin,request,updates,|writer| {
        live::write_raw_field(&original.frame,&original.field,writer,&proc.released).map(|_|())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn unavailable_storage_at_startup_retries_without_consuming_the_pipe_or_ending_the_agent() {
        let app=super::super::tests::ctx("bounded-startup-storage");let directory=app.data_dir().unwrap();let root=directory.join("chat-spool");
        std::fs::write(&root,"Temporarily unavailable").unwrap();let cancel=AtomicBool::new(false);let mut attempts=0;
        let archive=retry_archive(&directory,&cancel,&mut |error| {
            assert!(error.starts_with("storage_pressure:"));attempts+=1;
            std::fs::remove_file(&root).unwrap();Ok(())
        }).unwrap();assert_eq!(attempts,1);assert_eq!(archive.owned_bytes(),0);
        let canceled=AtomicBool::new(true);
        assert_eq!(retry_archive(&directory,&canceled,&mut |_|panic!("Canceled before opening storage")).err().unwrap(),"read_canceled");
        drop(archive);drop(app);std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn approved_projected_permission_echoes_the_complete_original_input() {
        let app=super::super::tests::ctx("bounded-permission-original");let (proc,stdout)=super::super::tests::cat_process();
        let manager=ChatManager::new();manager.sessions.lock().unwrap().insert("s".into(),proc.clone());
        let input=json!({"content":"雪\\\"😀\n".repeat(90_000),"lateNumber":719,"lateFlag":true});
        let raw=format!("{}\n",json!({"type":"control_request","request_id":"request-\"original","request":{"subtype":"can_use_tool","tool_name":"Write","input":input}}));
        let archive=live::Archive::at(&app.data_dir().unwrap()).unwrap();let mut receiver=live::Receiver::new(raw.as_bytes(),archive,"s",1);
        let (frame,record)=receiver.next(&proc.released,&mut |_|Ok(()),&mut |_|Ok(())).unwrap().unwrap();
        handle_record(&app,"s",&proc,&frame,&record);
        assert!(proc.permissions.lock().unwrap()["request-\"original"]["input"]["content"].as_str().unwrap().len()<=crate::agent::history_store::limits::PREVIEW_BYTES);
        assert!(manager.respond_permission(&app,"s","request-\"original",true,Some(record.value["request"]["input"].clone()),None,None).unwrap_err().starts_with("detail_requires_paging:"));
        assert!(proc.permissions.lock().unwrap().contains_key("request-\"original"));
        let reader=std::thread::spawn(move||{let mut text=String::new();BufReader::new(stdout).read_line(&mut text).unwrap();serde_json::from_str::<Value>(&text).unwrap()});
        let updates=json!([{"type":"addRules","rules":[]}]);
        manager.respond_permission(&app,"s","request-\"original",true,None,None,Some(updates.clone())).unwrap();
        let response=reader.join().unwrap();assert_eq!(response,protocol::permission_response("request-\"original",true,input,None,Some(updates)));
        assert!(proc.permissions.lock().unwrap().is_empty());assert!(proc.permission_originals.lock().unwrap().is_empty());
        proc.child.lock().unwrap().kill().unwrap();proc.child.lock().unwrap().wait().unwrap();
        let directory=app.data_dir().unwrap();drop(receiver);drop(manager);drop(proc);drop(app);std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn failed_permission_write_keeps_the_request_available_for_retry() {
        let app=super::super::tests::ctx("bounded-permission-retry");let proc=super::super::tests::inert_process(SessionKind::Claude);
        let manager=ChatManager::new();manager.sessions.lock().unwrap().insert("s".into(),proc.clone());
        let request=json!({"id":"request","subtype":"can_use_tool","tool_name":"Bash","input":{"command":"pwd"}});
        proc.permissions.lock().unwrap().insert("request".into(),request.clone());
        for allow in [true,false] {
            assert!(manager.respond_permission(&app,"s","request",allow,None,None,None).is_err());
            assert_eq!(proc.permissions.lock().unwrap().get("request"),Some(&request));
        }
        proc.child.lock().unwrap().wait().unwrap();
        let directory=app.data_dir().unwrap();drop(manager);drop(proc);drop(app);std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn bounded_user_original_is_saved_before_dispatch_and_preserves_queue_input() {
        let app=super::super::tests::ctx("bounded-user-original-before-dispatch");let (mut proc,stdout)=super::super::tests::cat_process();
        Arc::get_mut(&mut proc).unwrap().bounded_protocol=true;proc.ready.store(true,Ordering::Relaxed);
        app.chat().sessions.lock().unwrap().insert("s".into(),proc.clone());
        let original="雪😀 complete manual original\n".repeat(3000);let directory=app.data_dir().unwrap();let root=directory.join("chat-spool");
        let read_root=root.clone();let expected=original.clone();
        let reader=std::thread::spawn(move|| {
            let mut line=String::new();BufReader::new(stdout).read_line(&mut line).unwrap();
            let input:Value=serde_json::from_str(&line).unwrap();assert_eq!(input.pointer("/message/content/0/text").and_then(Value::as_str),Some(expected.as_str()));
            let entries=std::fs::read_dir(&read_root).unwrap().collect::<Result<Vec<_>,_>>().unwrap();
            let path=entries.iter().find(|entry|entry.path().extension().is_some_and(|extension|extension=="jsonl")).unwrap().path();
            let raw=std::fs::read_to_string(path).unwrap();let values=raw.lines().filter_map(|line|serde_json::from_str::<Value>(line).ok()).collect::<Vec<_>>();
            assert!(values.iter().any(|value|value["type"]=="proposal" && value["text"]==expected));
            assert!(values.iter().any(|value|value["type"]=="dispatch" && value["text"]==expected));
        });
        assert_eq!(app.chat().send(&app,"s",&original,vec![],"queue").unwrap(),"sent");reader.join().unwrap();
        proc.ready.store(false,Ordering::Relaxed);
        assert_eq!(app.chat().send(&app,"s","Queued original",vec![],"queue").unwrap(),"queued");
        let file=std::fs::read_dir(&root).unwrap().find(|entry|entry.as_ref().unwrap().path().extension().is_some_and(|extension|extension=="jsonl")).unwrap().unwrap().path();
        let values=std::fs::read_to_string(file).unwrap().lines().map(|line|serde_json::from_str::<Value>(line).unwrap()).collect::<Vec<_>>();
        assert!(values.iter().any(|value|value["type"]=="delivery" && value["state"]=="pipe_written"));
        assert!(values.iter().any(|value|value["type"]=="proposal" && value["text"]=="Queued original"));
        assert_eq!(values.iter().filter(|value|value["type"]=="dispatch").count(),1);
        proc.child.lock().unwrap().kill().unwrap();proc.child.lock().unwrap().wait().unwrap();
        app.chat().sessions.lock().unwrap().clear();drop(proc);drop(app);std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn unavailable_outgoing_storage_rejects_before_queue_or_provider_input_changes() {
        let app=super::super::tests::ctx("bounded-outgoing-storage-rejection");let (mut proc,stdout)=super::super::tests::cat_process();
        Arc::get_mut(&mut proc).unwrap().bounded_protocol=true;proc.ready.store(true,Ordering::Relaxed);
        app.chat().sessions.lock().unwrap().insert("s".into(),proc.clone());let directory=app.data_dir().unwrap();
        std::fs::write(directory.join("chat-spool"),"Unavailable owned storage").unwrap();
        let error=app.chat().send(&app,"s","Never written",vec![],"queue").unwrap_err();assert!(error.starts_with("storage_pressure:"));
        assert!(proc.timeline.lock().unwrap().rows.is_empty());assert!(!proc.turn.lock().unwrap().running);assert!(proc.turn.lock().unwrap().waiting.is_empty());
        proc.ready.store(false,Ordering::Relaxed);
        assert!(app.chat().send(&app,"s","Never queued",vec![],"queue").unwrap_err().starts_with("storage_pressure:"));assert!(proc.turn.lock().unwrap().waiting.is_empty());
        proc.child.lock().unwrap().kill().unwrap();proc.child.lock().unwrap().wait().unwrap();
        let mut output=String::new();BufReader::new(stdout).read_to_string(&mut output).unwrap();assert!(output.is_empty());
        app.chat().sessions.lock().unwrap().clear();drop(proc);drop(app);std::fs::remove_dir_all(directory).unwrap();
    }

}

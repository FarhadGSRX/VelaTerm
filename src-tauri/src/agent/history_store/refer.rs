//! End-to-end reference output with bounded backpressure and cancellation on connection close.

use std::io::{Read,Write};
use std::sync::{Arc,mpsc::{self,Receiver,SyncSender}};
use std::sync::atomic::{AtomicBool,AtomicUsize,Ordering};
use crate::{db::repo,host::AppCtx,models::SessionKind,agent::server::ReferRequest};
use super::{event_stream,prose::Prose};

static ACTIVE:AtomicUsize=AtomicUsize::new(0);
struct Slot;
impl Slot {
    fn acquire() -> Result<Self,(u16,String)> {
        ACTIVE.fetch_update(Ordering::SeqCst,Ordering::SeqCst,|count|(count<2).then_some(count+1))
            .map_err(|_|error(503,"index_busy: too many active reference streams"))?;Ok(Self)
    }
}
impl Drop for Slot {fn drop(&mut self) {ACTIVE.fetch_sub(1,Ordering::SeqCst);}}

fn error(code:u16,message:&str) -> (u16,String) {(code,serde_json::json!({"error":message}).to_string())}

pub struct Stream {receiver:Receiver<Result<Vec<u8>,String>>,pending:std::io::Cursor<Vec<u8>>,cancel:Arc<AtomicBool>,waiting:bool}
impl Read for Stream {
    fn read(&mut self,output:&mut [u8]) -> std::io::Result<usize> {
        if output.is_empty() {return Ok(0);}
        loop {
            let count=self.pending.read(output)?;if count>0 {return Ok(count);}
            match self.receiver.recv_timeout(std::time::Duration::from_millis(250)) {
                Ok(Ok(bytes))=>{self.waiting=false;self.pending=std::io::Cursor::new(bytes);},
                Ok(Err(error))=>return Err(std::io::Error::other(error)),
                Err(mpsc::RecvTimeoutError::Timeout)=>{
                    if self.waiting {output[0]=b'\n';return Ok(1);}
                },
                Err(mpsc::RecvTimeoutError::Disconnected)=>return Ok(0),
            }
        }
    }
}
impl Drop for Stream {fn drop(&mut self) {self.cancel.store(true,Ordering::Relaxed);}}

pub fn markdown_stream(branch:super::api::Branch,name:String,at:Option<String>) -> Stream {
    let (sender,receiver)=mpsc::sync_channel(2);let cancel=Arc::new(AtomicBool::new(false));let worker_cancel=cancel.clone();
    std::thread::spawn(move|| {
        let mut writer=std::io::BufWriter::with_capacity(32*1024,Output {sender:sender.clone(),cancel:worker_cancel.clone()});
        let result=super::markdown::write_markdown(branch,&name,at.as_deref(),&worker_cancel,&mut writer)
            .and_then(|_|writer.flush().map_err(|e|e.to_string()));
        if let Err(error)=result {let _=sender.send(Err(error));}
    });
    Stream {receiver,pending:std::io::Cursor::new(Vec::new()),cancel,waiting:false}
}

/// Finish HTTP chunking only after the source has finished successfully. The library encoder emits
/// its final chunk on Drop even after a reader error, which would otherwise certify a truncated read.
pub fn respond(request:crate::diagnostics::HttpRequest,response:tiny_http::Response<Stream>) -> std::io::Result<()> {
    request.respond_stream(response)
}

struct Output {sender:SyncSender<Result<Vec<u8>,String>>,cancel:Arc<AtomicBool>}
impl Write for Output {
    fn write(&mut self,bytes:&[u8]) -> std::io::Result<usize> {
        for chunk in bytes.chunks(32*1024) {
            if self.cancel.load(Ordering::Relaxed) {return Err(std::io::Error::other("read_canceled"));}
            self.sender.send(Ok(chunk.to_vec())).map_err(|_|std::io::Error::other("read_canceled"))?;
        }Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {Ok(())}
}

pub fn supported(app:&AppCtx,request:&ReferRequest) -> bool {
    let conn=app.db().conn.lock().unwrap();
    repo::resolve_session_ref(&conn,&request.target).ok().and_then(|found|match found {repo::SessionRefMatch::One(id)=>Some(id),_=>None})
        .and_then(|id|repo::get_session(&conn,&id).ok().flatten()).is_some_and(|session|
            matches!(session.kind,SessionKind::Codex|SessionKind::Claude|SessionKind::Pi|SessionKind::Omp))
}

pub fn response(app:&AppCtx,request:&ReferRequest,format:&str) -> Result<tiny_http::Response<Stream>,(u16,String)> {
    let slot=Slot::acquire()?;
    let session={
        let conn=app.db().conn.lock().map_err(|_|error(500,"database is unavailable"))?;
        let found=repo::resolve_session_ref(&conn,&request.target).map_err(|message|error(500,&message))?;
        let id=match found {
            repo::SessionRefMatch::One(id)=>id,
            repo::SessionRefMatch::None=>return Err(error(404,"no session matches the requested reference")),
            repo::SessionRefMatch::Ambiguous(candidates)=>return Err((409,serde_json::json!({"error":"ambiguous session reference",
                "candidates":candidates.into_iter().map(|(id,name)|serde_json::json!({"sessionId":id,"name":name})).collect::<Vec<_>>()} ).to_string())),
        };
        repo::get_session(&conn,&id).map_err(|message|error(500,&message))?.ok_or_else(||error(404,"session not found"))?
    };
    let cancel=Arc::new(AtomicBool::new(false));
    if session.agent_session_id.as_deref().is_none_or(|id|id.trim().is_empty()) {
        return Err(error(409,"This session has no captured native conversation ID"));
    }
    let request=request.clone();let app=app.clone();
    let json=format=="json";let (sender,receiver)=mpsc::sync_channel(2);let worker_cancel=cancel.clone();
    std::thread::spawn(move|| {
        let _slot=slot;let mut writer=std::io::BufWriter::with_capacity(32*1024,Output {sender:sender.clone(),cancel:worker_cancel.clone()});
        let result=(|| {
            let branch=event_stream::complete(&app,&session.id,&worker_cancel)?;
            let fallback_branch=branch.clone();let prose=Prose::new(branch,&worker_cancel)?;
            let (start,end)=crate::agent::server::refer_window(prose.total as usize,request.start,request.end,request.last);
            let mut metadata=serde_json::json!({"sessionId":session.id,"name":session.name,"kind":session.kind,"archived":session.archived_at.is_some(),
                "total":prose.total,"range":[start,end]});
            let prose=if let Some(question)=request.ask.as_deref().map(str::trim).filter(|text|!text.is_empty()) {
                match super::ask::answer(&app,&request,&session,prose,worker_cancel.clone()) {
                    Ok(answer)=>{
                        crate::agent::server::apply_answered(&mut metadata,question,answer);
                        if json {serde_json::to_writer(&mut writer,&metadata).map_err(|e|e.to_string())?;}
                        else {writer.write_all(crate::agent::cli_client::render_answer(&metadata).as_bytes()).map_err(|e|e.to_string())?;}
                        return writer.flush().map_err(|e|e.to_string());
                    },
                    Err(reason)=>{
                        if worker_cancel.load(Ordering::Relaxed) {return Err("read_canceled".into());}
                        metadata["askFailed"]=serde_json::json!(reason);
                        if !json {writeln!(writer,"vrefer: could not answer ({}); printing the transcript instead",metadata["askFailed"].as_str().unwrap()).map_err(|e|e.to_string())?;}
                        Prose::new(fallback_branch,&worker_cancel)?
                    },
                }
            } else {prose};
            if json {
                let encoded=metadata.to_string();writer.write_all(encoded[..encoded.len()-1].as_bytes()).map_err(|e|e.to_string())?;
                writer.write_all(b",\"messages\":").map_err(|e|e.to_string())?;
                prose.write_messages(start as u64,end as u64,&worker_cancel,&mut writer)?;
                writer.write_all(b"}").map_err(|e|e.to_string())?;
            } else {
                let id=metadata["sessionId"].as_str().unwrap_or_default().chars().take(8).collect::<String>();
                let window=if end>start {format!("showing #{start}-#{}",end-1)} else {"showing none".into()};
                write!(writer,"=== Session: {} ({id}) · {}{} · {} messages · {window} ===\n",metadata["name"].as_str().unwrap_or_default(),
                    metadata["kind"].as_str().unwrap_or_default(),if metadata["archived"]==true {" · archived"} else {""},prose.total).map_err(|e|e.to_string())?;
                prose.write_text(start as u64,end as u64,&worker_cancel,&mut writer)?;
                if start>0 {write!(writer,"\n(earlier messages: vrefer {id} --range 0:{start})\n").map_err(|e|e.to_string())?;}
            }
            writer.flush().map_err(|e|e.to_string())
        })();
        if let Err(error)=result {let _=sender.send(Err(error));}
    });
    let content_type=if json {"application/json; charset=utf-8"} else {"text/plain; charset=utf-8"};
    Ok(tiny_http::Response::new(tiny_http::StatusCode(200),vec![tiny_http::Header::from_bytes("Content-Type",content_type).unwrap(),
        tiny_http::Header::from_bytes("X-Vlx-History-Protocol","2").unwrap()],
        Stream {receiver,pending:std::io::Cursor::new(Vec::new()),cancel,waiting:true},None,None))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn source_failure_cannot_become_a_successfully_finished_http_stream() {
        let server=tiny_http::Server::http("127.0.0.1:0").unwrap();let address=server.server_addr().to_ip().unwrap();
        let (sender,receiver)=mpsc::sync_channel(2);let cancel=Arc::new(AtomicBool::new(false));
        sender.send(Ok(b"{\"messages\":[\"prefix".to_vec())).unwrap();sender.send(Err("source_changed: original was replaced".into())).unwrap();drop(sender);
        let worker=std::thread::spawn(move|| {
            let request=server.recv().unwrap();let stream=Stream {receiver,pending:std::io::Cursor::new(Vec::new()),cancel,waiting:false};
            respond(crate::diagnostics::HttpRequest::new(request),tiny_http::Response::new(tiny_http::StatusCode(200),Vec::new(),stream,None,None))
        });
        let agent=ureq::AgentBuilder::new().timeout(std::time::Duration::from_secs(2)).try_proxy_from_env(false).build();
        let mut response=agent.get(&format!("http://{address}/refer")).set("Connection","close").call().unwrap().into_reader();
        let mut output=Vec::new();assert!(response.read_to_end(&mut output).is_err());assert_eq!(output,b"{\"messages\":[\"prefix");
        assert!(worker.join().unwrap().is_err());
    }

    #[test]
    fn waiting_stream_checks_connection_liveness_without_changing_json_contents() {
        let (sender,receiver)=mpsc::sync_channel(2);let cancel=Arc::new(AtomicBool::new(false));
        let mut stream=Stream {receiver,pending:std::io::Cursor::new(Vec::new()),cancel:cancel.clone(),waiting:true};
        let mut byte=[0];assert_eq!(stream.read(&mut byte).unwrap(),1);assert_eq!(byte[0],b'\n');
        sender.send(Ok(b"{\"total\":1}".to_vec())).unwrap();drop(sender);
        let mut output=vec![b'\n'];stream.read_to_end(&mut output).unwrap();assert_eq!(serde_json::from_slice::<serde_json::Value>(&output).unwrap()["total"],1);
        drop(stream);assert!(cancel.load(Ordering::Relaxed));
    }

    #[test]
    fn disconnect_cancels_a_backpressured_stream_and_releases_its_worker() {
        let (sender,receiver)=mpsc::sync_channel(2);let cancel=Arc::new(AtomicBool::new(false));let worker_cancel=cancel.clone();
        let worker=std::thread::spawn(move|| {
            let mut output=Output {sender,cancel:worker_cancel};output.write_all(&vec![b'x';256*1024]).unwrap_err().to_string()
        });
        let stream=Stream {receiver,pending:std::io::Cursor::new(Vec::new()),cancel:cancel.clone(),waiting:false};drop(stream);
        assert!(cancel.load(Ordering::Relaxed));assert!(worker.join().unwrap().contains("read_canceled"));
    }
}

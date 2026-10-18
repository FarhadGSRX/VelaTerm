//! Explicit client/native user identities, gated by the installed app-server schema.

use super::*;
use std::path::{Path,PathBuf};
use std::sync::OnceLock;
use std::time::{Instant,SystemTime};

const SCHEMA_BYTES:u64=256*1024;
const PROBE_TIME:Duration=Duration::from_secs(3);
type Capabilities=HashMap<(String,u64,Option<SystemTime>),bool>;
static CAPABILITIES:OnceLock<Mutex<Capabilities>>=OnceLock::new();

struct ProbeDirectory(PathBuf);
impl Drop for ProbeDirectory {
    fn drop(&mut self) {let _=std::fs::remove_dir_all(&self.0);}
}
fn schema(path:&Path) -> Option<Value> {
    use std::io::Read;
    let file=std::fs::File::open(path).ok()?;
    if file.metadata().ok()?.len()>SCHEMA_BYTES {return None;}
    let mut bytes=Vec::new();file.take(SCHEMA_BYTES+1).read_to_end(&mut bytes).ok()?;
    if bytes.len() as u64>SCHEMA_BYTES {return None;}
    serde_json::from_slice(&bytes).ok()
}
fn has_client_ids(directory:&Path) -> bool {
    let Some(start)=schema(&directory.join("v2/TurnStartParams.json")) else {return false;};
    let Some(steer)=schema(&directory.join("v2/TurnSteerParams.json")) else {return false;};
    let Some(item)=schema(&directory.join("v2/ItemStartedNotification.json")) else {return false;};
    start.pointer("/properties/clientUserMessageId").is_some()
        && steer.pointer("/properties/clientUserMessageId").is_some()
        && item.pointer("/definitions/ThreadItem/oneOf").and_then(Value::as_array).is_some_and(|items|items.iter().any(|item| {
            item.pointer("/properties/type/enum/0").and_then(Value::as_str)==Some("userMessage")
                && item.pointer("/properties/clientId").is_some() && item.pointer("/properties/id").is_some()
        }))
}
pub(super) fn supported(bin:&str) -> bool {
    // Schema generation is an offline CLI operation. It cannot create, resume, or submit a turn.
    // Unknown wrappers and older CLIs retain their previous request shape and unconfirmed identity.
    if bin.len()>4096 {return false;}
    let Ok(metadata)=std::fs::metadata(bin) else {return false;};
    let key=(bin.to_owned(),metadata.len(),metadata.modified().ok());
    let mut cache=CAPABILITIES.get_or_init(||Mutex::new(HashMap::new())).lock().unwrap();
    if let Some(value)=cache.get(&key) {return *value;}
    let directory=ProbeDirectory(std::env::temp_dir().join(format!("vlx-codex-schema-{}",uuid::Uuid::new_v4())));
    let result=(|| {
        std::fs::create_dir(&directory.0).ok()?;
        let mut command=crate::host::command(bin);
        command.args(["app-server","generate-json-schema","--out"]).arg(&directory.0)
            .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        crate::agent::executable::prepare_command(&mut command,bin);
        #[cfg(unix)] {use std::os::unix::process::CommandExt;command.process_group(0);}
        let mut child=command.spawn().ok()?;let started=Instant::now();
        loop {match child.try_wait() {
            Ok(Some(status))=>return Some(status.success() && has_client_ids(&directory.0)),
            Ok(None) if started.elapsed()<PROBE_TIME=>std::thread::park_timeout(Duration::from_millis(10)),
            _=>{terminate_owned(&mut child);let _=child.wait();return None;},
        }}
    })().unwrap_or(false);
    if cache.len()>=16 {cache.clear();}
    cache.insert(key,result);result
}

pub(super) fn client(proc:&ChatProcess,row:&str,thread:&str) -> Result<Option<String>,String> {
    if !proc.codex_client_ids {return Ok(None);}
    if row.is_empty() || row.len()>200 || thread.is_empty() || thread.len()>200 {return Err("unsupported_record: client user identity is invalid".into());}
    use sha2::{Digest,Sha256};
    let digest=Sha256::digest(serde_json::to_vec(&(thread,proc.started_at,row)).unwrap());
    let id=digest.iter().map(|byte|format!("{byte:02x}")).collect::<String>();
    let mut pending=proc.codex_pending_user_ids.lock().unwrap();
    if !pending.contains_key(&id) && pending.len()>=128 {return Err("index_busy: waiting for earlier user message identity confirmations".into());}
    pending.insert(id.clone(),row.to_owned());Ok(Some(id))
}
pub(super) fn request(mut request:Value,client:Option<&str>) -> Value {
    if let Some(client)=client {request["params"]["clientUserMessageId"]=json!(client);}
    request
}

pub(super) fn observe(app:&AppCtx,session:&str,proc:&Arc<ChatProcess>,params:&Value) {
    let item=&params["item"];
    if !proc.codex_client_ids || item["type"]!="userMessage" {return;}
    let Some(client)=item["clientId"].as_str().filter(|id|!id.is_empty() && id.len()<=200) else {return;};
    let Some(native)=item["id"].as_str().filter(|id|!id.is_empty() && id.len()<=200) else {return;};
    let Some(turn)=params["turnId"].as_str().filter(|id|!id.is_empty() && id.len()<=200) else {return;};
    let Some(thread)=params["threadId"].as_str() else {return;};
    if proc.agent_session_id.lock().unwrap().as_deref()!=Some(thread) {return;}
    let Some(row)=proc.codex_pending_user_ids.lock().unwrap().remove(client) else {return;};
    let mut targets=proc.user_targets.lock().unwrap();
    // A provider event names the exact client id. It does not infer a match from the text or turn order.
    if let Some(previous)=targets.get(&row) {
        if previous.message_id==native && previous.turn_id.as_deref()==Some(turn) {return;}
        if previous.message_id!=previous.turn_id.as_deref().unwrap_or_default() {
            emit(app,session,json!({"type":"historyReadError","message":"index_incomplete: the provider returned conflicting user message identities"}));return;
        }
    }
    targets.insert(row.clone(),super::super::history::RewindTarget {
        message_id:native.into(),last_message_id:None,turn_id:Some(turn.into()),text:String::new(),
    });drop(targets);
    bounded::identity(app,session,proc,&row,native,turn);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn installed_schema_must_explicitly_support_all_three_identity_fields() {
        let directory=ProbeDirectory(std::env::temp_dir().join(format!("vlx-codex-schema-test-{}",uuid::Uuid::new_v4())));
        std::fs::create_dir_all(directory.0.join("v2")).unwrap();
        let start=directory.0.join("v2/TurnStartParams.json");let steer=directory.0.join("v2/TurnSteerParams.json");
        let item=directory.0.join("v2/ItemStartedNotification.json");
        std::fs::write(&start,r#"{"properties":{"clientUserMessageId":{}}}"#).unwrap();
        std::fs::write(&steer,r#"{"properties":{"clientUserMessageId":{}}}"#).unwrap();
        assert!(!has_client_ids(&directory.0));
        std::fs::write(&item,r#"{"definitions":{"ThreadItem":{"oneOf":[{"properties":{"type":{"enum":["userMessage"]},"id":{},"clientId":{}}}]}}}"#).unwrap();
        assert!(has_client_ids(&directory.0));
        std::fs::write(&steer,r#"{"properties":{}}"#).unwrap();assert!(!has_client_ids(&directory.0));
        std::fs::write(&item,vec![b' ';SCHEMA_BYTES as usize+1]).unwrap();assert!(schema(&item).is_none());
    }

    #[test]
    fn repeated_text_and_same_turn_use_distinct_native_messages_and_reject_foreign_events() {
        let app=super::super::tests::ctx("codex-explicit-user-ids");let mut proc=super::super::tests::inert_process(SessionKind::Codex);
        Arc::get_mut(&mut proc).unwrap().codex_client_ids=true;
        *proc.agent_session_id.lock().unwrap()=Some("root".into());
        for (client,native,thread) in [("first","native-one","root"),("second","native-two","root"),("foreign","native-three","child")] {
            let wire_client=self::client(&proc,client,"root").unwrap().unwrap();
            let params=json!({"threadId":thread,"turnId":"same-turn","item":{"type":"userMessage","id":native,"clientId":wire_client,"content":[{"type":"text","text":"Identical prompt"}]}});
            observe(&app,"s",&proc,&params);observe(&app,"s",&proc,&params);
        }
        let targets=proc.user_targets.lock().unwrap();assert_eq!(targets.len(),2);
        assert_eq!(targets["first"].message_id,"native-one");assert_eq!(targets["second"].message_id,"native-two");drop(targets);
        let client=client(&proc,"first","root").unwrap();
        assert_eq!(request(codex_protocol::turn_steer("r","root","same-turn","Prompt",&[]),client.as_deref())["params"]["clientUserMessageId"].as_str(),client.as_deref());
        Arc::get_mut(&mut proc).unwrap().codex_client_ids=false;
        assert!(request(codex_protocol::turn_steer("r","root","same-turn","Prompt",&[]),self::client(&proc,"first","root").unwrap().as_deref())["params"].get("clientUserMessageId").is_none());
        proc.child.lock().unwrap().wait().unwrap();let directory=app.data_dir().unwrap();drop(proc);drop(app);std::fs::remove_dir_all(directory).unwrap();
    }
}

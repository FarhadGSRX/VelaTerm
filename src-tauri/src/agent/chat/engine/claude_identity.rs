//! Correlate Claude's replay acknowledgement with the UUID sent on that exact user message.

use super::*;
use sha2::{Digest,Sha256};

fn native_id(session:&str,epoch:u64,client:&str) -> String {
    let digest=Sha256::digest(serde_json::to_vec(&(session,epoch,client)).unwrap());
    let mut bytes:[u8;16]=digest[..16].try_into().unwrap();bytes[6]=(bytes[6]&0x0f)|0x40;bytes[8]=(bytes[8]&0x3f)|0x80;
    uuid::Uuid::from_bytes(bytes).to_string()
}
/// `preset` is a UUID the message must carry for another reason, such as its recorded translation.
fn register(session:&str,proc:&ChatProcess,client:&str,preset:Option<String>) -> Result<String,String> {
    let native=preset.unwrap_or_else(||native_id(session,proc.started_at,client));
    let mut pending=proc.claude_user_ids.lock().unwrap();
    if !pending.contains_key(&native) && pending.len()>=128 {return Err("index_busy: waiting for earlier user message identity confirmations".into());}
    pending.insert(native.clone(),client.to_owned());drop(pending);
    Ok(native)
}
pub(super) fn write(session:&str,proc:&ChatProcess,client:&str,text:&str,images:&[ChatImage],preset:Option<String>) -> Result<(),String> {
    let native=register(session,proc,client,preset)?;
    proc.write_source(|writer|outgoing_wire::claude(writer,text,images,Some(&native)))
}
#[cfg(test)]
fn message(session:&str,proc:&ChatProcess,client:&str,text:&str,images:&[ChatImage]) -> Result<Value,String> {
    let native=register(session,proc,client,None)?;let mut message=protocol::user_message(text,images);message["uuid"]=json!(native);Ok(message)
}

pub(super) fn observe(app:&AppCtx,session:&str,proc:&Arc<ChatProcess>,value:&Value) {
    if value["type"]!="user" || !value["parent_tool_use_id"].is_null() {return;}
    let Some(native)=value["uuid"].as_str().filter(|id|uuid::Uuid::parse_str(id).is_ok()) else {return;};
    if value["session_id"].as_str().is_some_and(|id|proc.agent_session_id.lock().unwrap().as_deref().is_some_and(|native|native!=id)) {return;}
    let Some(client)=proc.claude_user_ids.lock().unwrap().remove(native) else {return;};
    proc.user_targets.lock().unwrap().insert(client.clone(),super::super::history::RewindTarget {
        message_id:native.into(),last_message_id:None,turn_id:None,text:String::new(),
    });bounded::identity(app,session,proc,&client,native,"");
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn acknowledgements_confirm_the_sent_uuid_and_preserve_repeated_prompts_as_distinct_messages() {
        let app=super::super::tests::ctx("claude-explicit-user-ids");let proc=super::super::tests::inert_process(SessionKind::Claude);
        *proc.agent_session_id.lock().unwrap()=Some("thread".into());
        let first=message("s",&proc,"first","Identical prompt",&[]).unwrap();let second=message("s",&proc,"second","Identical prompt",&[]).unwrap();
        assert_ne!(first["uuid"],second["uuid"]);assert_eq!(first,message("s",&proc,"first","Identical prompt",&[]).unwrap());
        assert!(proc.user_targets.lock().unwrap().is_empty());
        for (client,input) in [("first",first),("second",second)] {
            let mut acknowledgement=input.clone();acknowledgement["session_id"]=json!("foreign");observe(&app,"s",&proc,&acknowledgement);
            assert!(!proc.user_targets.lock().unwrap().contains_key(client));
            acknowledgement["session_id"]=json!("thread");acknowledgement["parent_tool_use_id"]=json!("child");observe(&app,"s",&proc,&acknowledgement);
            assert!(!proc.user_targets.lock().unwrap().contains_key(client));
            acknowledgement["parent_tool_use_id"]=Value::Null;observe(&app,"s",&proc,&acknowledgement);observe(&app,"s",&proc,&acknowledgement);
            assert_eq!(proc.user_targets.lock().unwrap()[client].message_id,input["uuid"].as_str().unwrap());
        }
        assert!(proc.claude_user_ids.lock().unwrap().is_empty());
        for index in 0..128 {message("s",&proc,&format!("pending-{index}"),"Pending",&[]).unwrap();}
        assert!(message("s",&proc,"over-capacity","Pending",&[]).unwrap_err().starts_with("index_busy:"));
        assert_ne!(native_id("s",proc.started_at,"first"),native_id("other",proc.started_at,"first"));
        assert_ne!(native_id("s",proc.started_at,"first"),native_id("s",proc.started_at+1,"first"));
        proc.child.lock().unwrap().wait().unwrap();let directory=app.data_dir().unwrap();drop(proc);drop(app);std::fs::remove_dir_all(directory).unwrap();
    }
}

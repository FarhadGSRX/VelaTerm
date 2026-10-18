//! Compare native identity and complete display fields without deleting a unique original or ledger.
//! Control facts, outgoing deliveries, and child scopes require their own reconciliation before a
//! matching display source can authorize full-context selection or original garbage collection.

use std::collections::{BTreeMap,HashMap,HashSet};
use std::io::Read;
use std::sync::atomic::{AtomicBool,Ordering};
use rusqlite::{params,Connection,OptionalExtension};
use serde_json::{json,Value};
use sha2::{Digest,Sha256};
use crate::models::SessionKind;
use super::{api::{self,Branch},store::{RegisteredSource,sql}};

fn canceled(cancel:&AtomicBool) -> Result<(),String> {
    if cancel.load(Ordering::Relaxed) {Err("read_canceled".into())} else {Ok(())}
}
fn string_hash(conn:&Connection,source:&RegisteredSource,reference:&str,cancel:&AtomicBool) -> Result<Vec<u8>,String> {
    let mut hash=Sha256::new();let mut sequence=0;
    loop {canceled(cancel)?;let (text,_,end,size)=api::read_piece(conn,source,reference,sequence,0)?;
        hash.update(text.as_bytes());if end>=size {break;}sequence+=1;
    }Ok(hash.finalize().to_vec())
}
fn canonical(conn:&Connection,source:&RegisteredSource,reference:&str,cancel:&AtomicBool) -> Result<Vec<u8>,String> {
    let (start,path,encoding):(u64,String,String)=conn.query_row("SELECT record_start,path,encoding FROM fields WHERE ref=?1 AND source_id=?2 AND generation=?3",
        params![reference,source.id,source.generation],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).map_err(sql)?;
    let raw:String=conn.query_row("SELECT projection FROM records WHERE source_id=?1 AND generation=?2 AND start=?3 AND active=1",
        params![source.id,source.generation,start],|row|row.get(0)).map_err(sql)?;
    let record:Value=serde_json::from_str(&raw).map_err(|_|"index_incomplete: reconciliation projection is invalid")?;
    let mut value=record.pointer(&path).cloned().ok_or("index_incomplete: reconciliation field has no semantic projection")?;
    if encoding=="string" && value.is_string() {return Ok(Sha256::digest(json!(["string",string_hash(conn,source,reference,cancel)?]).to_string().as_bytes()).to_vec());}
    let prefix=if encoding=="string" {format!("{path}/@json")} else {path};
    // The projector preserves the bounded container shape, scalar values, and all original string
    // references. Hash each full decoded string; canonical object key order comes from serde's map.
    let mut statement=conn.prepare("SELECT ref,path FROM fields WHERE source_id=?1 AND generation=?2 AND record_start=?3 AND encoding='string' ORDER BY path").map_err(sql)?;
    let mut fields=statement.query(params![source.id,source.generation,start]).map_err(sql)?;
    while let Some(field)=fields.next().map_err(sql)? {
        canceled(cancel)?;let path:String=field.get(1).map_err(sql)?;
        let Some(relative)=path.strip_prefix(&prefix).filter(|relative|relative.is_empty() || relative.starts_with('/')) else {continue;};
        if let Some(target)=value.pointer_mut(relative) {if target.is_string() {
            let reference:String=field.get(0).map_err(sql)?;*target=json!(["string",string_hash(conn,source,&reference,cancel)?]);
        }}
    }
    Ok(Sha256::digest(json!(["json",value]).to_string().as_bytes()).to_vec())
}

struct Field {reference:String,path:String,encoding:String,append:bool,image:Option<(String,u64)>}
fn fields(conn:&Connection,source:&RegisteredSource,row:&str) -> Result<BTreeMap<String,Vec<Field>>,String> {
    let mut statement=conn.prepare("SELECT rf.label,f.ref,f.path,f.encoding,EXISTS(SELECT 1 FROM append_fields a WHERE a.ref=f.ref) FROM row_fields rf
        JOIN fields f ON f.ref=rf.ref AND f.source_id=rf.source_id AND f.generation=rf.generation
        JOIN records r ON r.source_id=f.source_id AND r.generation=f.generation AND r.start=f.record_start
        WHERE rf.source_id=?1 AND rf.generation=?2 AND rf.row_id=?3 AND r.active=1 AND (?4 IS NULL OR r.end<=?4)
        AND (?5 IS NULL OR r.ordinal IS NULL OR r.ordinal<?5) ORDER BY rf.sequence").map_err(sql)?;
    let mut rows=statement.query(params![source.id,source.generation,row,source.segment.end_byte,source.segment.end_ordinal]).map_err(sql)?;
    let mut result:BTreeMap<String,Vec<Field>>=BTreeMap::new();let mut count=0;let mut bytes=0;
    while let Some(row)=rows.next().map_err(sql)? {
        let mut label:String=row.get(0).map_err(sql)?;
        if label=="original" {continue;}
        let image=if let Some(metadata)=label.strip_prefix("attachment:") {
            let (mime,prefix)=metadata.rsplit_once(':').ok_or("index_incomplete: reconciliation image metadata is invalid")?;
            let prefix=prefix.parse().map_err(|_|"index_incomplete: reconciliation image boundary is invalid")?;
            Some((mime.to_owned(),prefix))
        } else {None};
        if image.is_some() {label="attachments".into();}
        let field=Field {reference:row.get(1).map_err(sql)?,path:row.get(2).map_err(sql)?,encoding:row.get(3).map_err(sql)?,append:row.get(4).map_err(sql)?,image};
        count+=1;bytes+=field.path.len()+field.reference.len()+label.len();
        if count>super::limits::MAX_FIELDS || bytes>16*1024*1024 {return Err("index_incomplete: reconciliation metadata exceeds its byte budget".into());}
        result.entry(label).or_default().push(field);
    }
    for fields in result.values_mut() {
        let roots=fields.iter().map(|field|(field.reference.clone(),field.path.clone())).collect::<Vec<_>>();
        fields.retain(|field|!roots.iter().any(|(reference,path)|*reference!=field.reference && field.path.starts_with(&(path.clone()+"/"))));
    }Ok(result)
}
fn signature(conn:&Connection,source:&RegisteredSource,row:&str,cancel:&AtomicBool) -> Result<BTreeMap<String,Vec<u8>>,String> {
    let groups=fields(conn,source,row)?;let mut result=BTreeMap::new();
    for (label,fields) in groups {
        canceled(cancel)?;
        if label=="attachments" {
            let mut hash=Sha256::new();hash.update((fields.len() as u64).to_le_bytes());
            for field in fields {
                let (mime,prefix)=field.image.ok_or("index_incomplete: reconciliation image metadata is unavailable")?;
                let reader=api::DecodedFieldReader::new(conn,source,&field.reference,prefix,None,0)?;
                let mut reader=base64::read::DecoderReader::new(reader,&base64::engine::general_purpose::STANDARD);
                let mut buffer=[0;super::limits::CHECKPOINT_BYTES];let mut image=Sha256::new();let mut length=0u64;
                loop {canceled(cancel)?;let bytes=reader.read(&mut buffer).map_err(|_|"index_incomplete: reconciliation image original is invalid")?;
                    if bytes==0 {break;}image.update(&buffer[..bytes]);length+=bytes as u64;}
                hash.update((mime.len() as u64).to_le_bytes());hash.update(mime.as_bytes());hash.update(length.to_le_bytes());hash.update(image.finalize());
            }
            // Native data URLs and outgoing image fields can use different envelope lengths.
            // Their media type and complete decoded bytes, rather than that envelope, must agree.
            result.insert(label,hash.finalize().to_vec());continue;
        }
        if label=="body" || label=="output" {
            if fields.iter().all(|field|field.encoding=="string") {
                let mut hash=Sha256::new();
                for (index,field) in fields.iter().enumerate() {
                    if index>0 && !field.append {hash.update(b"\n");}
                    let mut sequence=0;loop {canceled(cancel)?;
                        let (text,_,end,size)=api::read_piece(conn,source,&field.reference,sequence,0)?;hash.update(text.as_bytes());if end>=size {break;}sequence+=1;
                    }
                }result.insert(label,hash.finalize().to_vec());continue;
            }
        }
        let mut values=BTreeMap::new();
        for field in &fields {
            let name=if fields.len()==1 {""} else {field.path.rsplit('/').next().unwrap_or_default()};
            if values.insert(name.to_owned(),canonical(conn,source,&field.reference,cancel)?).is_some() {
                return Err("index_incomplete: repeated semantic fields require explicit reconciliation".into());
            }
        }result.insert(label,Sha256::digest(serde_json::to_vec(&values).map_err(|e|e.to_string())?).to_vec());
    }Ok(result)
}

/// Select native transcript sources only after every published protocol row has an exact original
/// counterpart. This authorizes conversation reads; it never authorizes deleting controller evidence.
pub fn native_context(branch:&Branch,cancel:&AtomicBool) -> Result<Branch,String> {
    canceled(cancel)?;super::children::validate_parent(branch)?;
    let conn=branch.store.connect()?;let mut protocols=HashSet::new();
    for source in &branch.sources {
        let scope:Option<String>=conn.query_row("SELECT session_scope FROM protocol_sources WHERE source_id=?1",[&source.id],|row|row.get(0)).optional().map_err(sql)?;
        if let Some(scope)=scope {if scope!=branch.scope {return Err("detail_unavailable: protocol original belongs to another session".into());}protocols.insert(source.id.clone());}
    }
    if protocols.is_empty() {return Ok(branch.clone());}
    if !matches!(branch.kind,SessionKind::Codex|SessionKind::Claude) {
        return Err("index_incomplete: this provider's protocol originals are awaiting native reconciliation".into());
    }
    let matched=matching_display_sources(branch,cancel)?;
    for source in branch.sources.iter().filter(|source|protocols.contains(&source.id)) {
        let rows:u64=conn.query_row("SELECT count(*) FROM rows r WHERE source_id=?1 AND generation=?2
            AND EXISTS(SELECT 1 FROM records p WHERE p.source_id=r.source_id AND p.generation=r.generation AND p.start=r.record_start AND p.active=1)
            AND (?3 IS NULL OR record_end<=?3) AND (?4 IS NULL OR ordinal IS NULL OR ordinal<?4)",
            params![source.id,source.generation,source.segment.end_byte,source.segment.end_ordinal],|row|row.get(0)).map_err(sql)?;
        if rows>0 && !matched.contains(&source.id) {
            return Err("index_incomplete: protocol conversation originals have no complete native counterpart".into());
        }
        let direction:Option<String>=conn.query_row("SELECT value FROM protocol_state WHERE source_id=?1 AND generation=?2 AND key='direction'",
            params![source.id,source.generation],|row|row.get(0)).optional().map_err(sql)?;
        let mut statement=conn.prepare("SELECT start,end,projection,error FROM records WHERE source_id=?1 AND generation=?2 AND active=1
            AND (?3 IS NULL OR end<=?3) AND (?4 IS NULL OR ordinal IS NULL OR ordinal<?4) ORDER BY start").map_err(sql)?;
        let mut records=statement.query(params![source.id,source.generation,source.segment.end_byte,source.segment.end_ordinal]).map_err(sql)?;
        while let Some(record)=records.next().map_err(sql)? {
            canceled(cancel)?;
            let error:Option<String>=record.get(3).map_err(sql)?;if let Some(error)=error {return Err(format!("index_incomplete: {error}"));}
            let raw:String=record.get(2).map_err(sql)?;let value:Value=serde_json::from_str(&raw).map_err(|_|"index_incomplete: protocol reconciliation projection is invalid")?;
            if direction.as_deref()==Some("outgoing") {
                match value["type"].as_str() {
                    Some("proposal"|"delivery"|"identity")=>{},
                    Some("dispatch")=> {
                        let id=value["id"].as_str().ok_or("index_incomplete: outgoing dispatch identity is missing")?;
                        let written:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM records WHERE source_id=?1 AND generation=?2
                            AND event_type='delivery' AND json_extract(projection,'$.id')=?3 AND json_extract(projection,'$.state')='pipe_written'
                            AND start>?4 AND (?5 IS NULL OR end<=?5))",params![source.id,source.generation,id,record.get::<_,u64>(0).map_err(sql)?,source.segment.end_byte],|row|row.get(0)).map_err(sql)?;
                        if !written {return Err("index_incomplete: outgoing pipe delivery is not durably confirmed".into());}
                    },
                    _=>return Err("index_incomplete: outgoing protocol record has no reconciliation contract".into()),
                }
            } else if branch.kind==SessionKind::Claude {
                if !matches!(value["type"].as_str(),Some("assistant"|"user"|"stream_event"|"system"|"result"|"control_request"|"control_response"
                    |"control_cancel_request"|"keep_alive"|"tool_progress"|"tool_use_summary"|"rate_limit_event"|"auth_status"
                    |"task_started"|"task_progress"|"task_notification"|"prompt_suggestion")) {
                    return Err("index_incomplete: incoming protocol record has no reconciliation contract".into());
                }
            } else if let Some(method)=value["method"].as_str() {
                if !matches!(method,"item/started"|"item/completed"|"item/agentMessage/delta"|"item/reasoning/textDelta"|"item/reasoning/summaryTextDelta"
                    |"item/reasoning/summaryPartAdded"|"item/commandExecution/outputDelta"|"item/plan/delta"|"item/commandExecution/terminalInteraction"
                    |"turn/started"|"turn/completed"|"turn/diff/updated"|"turn/plan/updated"|"thread/started"|"thread/status/changed"
                    |"thread/tokenUsage/updated"|"thread/name/updated"|"thread/archived"|"thread/unarchived"|"thread/closed"
                    |"error"|"warning"|"configWarning"|"deprecationNotice"|"model/rerouted"|"contextCompacted"|"account/updated"
                    |"account/rateLimits/updated"|"item/commandExecution/requestApproval"|"item/fileChange/requestApproval"
                    |"item/permissions/requestApproval"|"item/tool/requestUserInput"|"item/tool/call"|"serverRequest/resolved") {
                    return Err("index_incomplete: incoming protocol record has no reconciliation contract".into());
                }
            } else if value.get("id").is_none() || value.get("result").is_none() && value.get("error").is_none() {
                return Err("index_incomplete: incoming protocol record has no reconciliation contract".into());
            }
        }
    }
    let sources=branch.sources.iter().filter(|source|!protocols.contains(&source.id)).cloned().collect::<Vec<_>>();
    if sources.is_empty() {return Err("index_incomplete: the native transcript is unavailable for full-context reads".into());}
    Ok(Branch {sources,..branch.clone()})
}

pub fn matching_display_sources(branch:&Branch,cancel:&AtomicBool) -> Result<HashSet<String>,String> {
    canceled(cancel)?;let conn=branch.store.connect()?;let mut protocols=HashMap::new();
    for source in &branch.sources {
        let scope:Option<String>=conn.query_row("SELECT session_scope FROM protocol_sources WHERE source_id=?1",[&source.id],|row|row.get(0)).optional().map_err(sql)?;
        if let Some(scope)=scope {if scope!=branch.scope {return Err("detail_unavailable: protocol original belongs to another session".into());}protocols.insert(source.id.clone(),());}
    }
    let mut verified=HashSet::new();
    for source in &branch.sources {
        if !protocols.contains_key(&source.id) {continue;}
        if !matches!(branch.kind,SessionKind::Codex|SessionKind::Claude) {continue;}
        let native=branch.sources.iter().filter(|native|!protocols.contains_key(&native.id) && native.segment.native_id==source.segment.native_id).collect::<Vec<_>>();
        if native.is_empty() {continue;}
        let mut rows=conn.prepare("SELECT id,summary FROM rows r WHERE r.source_id=?1 AND r.generation=?2
            AND EXISTS(SELECT 1 FROM records p WHERE p.source_id=r.source_id AND p.generation=r.generation AND p.start=r.record_start AND p.active=1)
            AND (?3 IS NULL OR r.record_end<=?3) AND (?4 IS NULL OR r.ordinal IS NULL OR r.ordinal<?4) ORDER BY order_key").map_err(sql)?;
        let mut rows=rows.query(params![source.id,source.generation,source.segment.end_byte,source.segment.end_ordinal]).map_err(sql)?;let mut matched=true;let mut count=0;
        while let Some(row)=rows.next().map_err(sql)? {
            count+=1;
            canceled(cancel)?;let id:String=row.get(0).map_err(sql)?;let summary:String=row.get(1).map_err(sql)?;let summary:Value=serde_json::from_str(&summary).map_err(|e|e.to_string())?;
            let original=signature(&conn,source,&id,cancel)?;let mut found=false;
            if original.is_empty() {matched=false;break;}
            for native in &native {
                let raw:Option<String>=conn.query_row("SELECT summary FROM rows r WHERE r.source_id=?1 AND r.generation=?2 AND r.id=?3
                    AND EXISTS(SELECT 1 FROM records p WHERE p.source_id=r.source_id AND p.generation=r.generation AND p.start=r.record_start AND p.active=1)
                    AND (?4 IS NULL OR r.record_end<=?4) AND (?5 IS NULL OR r.ordinal IS NULL OR r.ordinal<?5)",
                    params![native.id,native.generation,id,native.segment.end_byte,native.segment.end_ordinal],|row|row.get(0)).optional().map_err(sql)?;
                let Some(raw)=raw else {continue;};let summary_native:Value=serde_json::from_str(&raw).map_err(|e|e.to_string())?;
                if ["kind","name","status","exitCode"].iter().any(|key|summary[*key]!=summary_native[*key])
                    || ["isError","streaming","stdoutTruncated","stderrTruncated"].iter().any(|key|summary[*key].as_bool().unwrap_or(false)!=summary_native[*key].as_bool().unwrap_or(false)) {continue;}
                if original==signature(&conn,native,&id,cancel)? {found=true;break;}
            }
            if !found {matched=false;break;}
        }
        if matched && count>0 {verified.insert(source.id.clone());}
    }Ok(verified)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn indexed(temp:&super::super::tests::Temp,kind:SessionKind,protocol:&str,native:&str) -> Branch {
        let archive=super::super::live::Archive::at(&temp.0).unwrap();let cancel=AtomicBool::new(false);
        let mut receiver=super::super::live::Receiver::new(protocol.as_bytes(),archive,"scope",7);let mut frame=None;
        while let Some((next,_))=receiver.next(&cancel,&mut |_|Ok(()),&mut |_|panic!("Storage is available")).unwrap() {frame=Some(next);}
        let frame=frame.unwrap();let store=super::super::store::Store::at(&temp.0).unwrap();
        let protocol=super::super::protocol_source::register(&store,kind,&frame.reference,"scope",7,"thread").unwrap();store.scan(kind,&protocol,&cancel,false).unwrap();
        let path=temp.0.join("native.jsonl");std::fs::write(&path,native).unwrap();
        let native=store.register(kind,super::super::source::Segment {path,native_id:"thread".into(),end_byte:None,end_ordinal:None}).unwrap();store.scan(kind,&native,&cancel,false).unwrap();
        Branch {store,sources:vec![native,protocol],generation:"branch".into(),kind,native_id:"thread".into(),error:None,scope:"scope".into(),parent:None}
    }
    #[test]
    fn same_native_identity_requires_full_original_bodies_and_preserves_unique_control_originals() {
        for kind in [SessionKind::Codex,SessionKind::Claude] {for matches in [true,false] {
            let temp=super::super::tests::Temp::new();let text="雪😀 original body\n".repeat(10_000);let other=if matches {text.clone()} else {text.clone()+"A late conflicting suffix"};
            let (protocol,native)=if kind==SessionKind::Codex {
                (json!({"method":"item/completed","params":{"threadId":"thread","item":{"id":"answer","type":"agentMessage","text":text}}}),
                    json!({"type":"event_msg","payload":{"type":"item_completed","item":{"id":"answer","type":"agent_message","content":other}}}))
            } else {
                (json!({"type":"assistant","uuid":"answer","message":{"id":"message","content":[{"type":"text","text":text}]}}),
                    json!({"type":"assistant","uuid":"answer","message":{"id":"message","content":[{"type":"text","text":other}]}}))
            };
            let protocol=protocol.to_string()+"\n";let native=native.to_string()+"\n";let branch=indexed(&temp,kind,&protocol,&native);
            let result=matching_display_sources(&branch,&AtomicBool::new(false)).unwrap();assert_eq!(result.contains(&branch.sources[1].id),matches,"{kind:?}");
            assert_eq!(std::fs::read_to_string(&branch.sources[0].segment.path).unwrap(),native);assert_eq!(std::fs::read_to_string(&branch.sources[1].segment.path).unwrap(),protocol);
            let selected=super::super::event_stream::Cursor::new(branch.clone(),&AtomicBool::new(false));
            assert_eq!(selected.is_ok(),matches,"{kind:?}");
            if let Ok(mut selected)=selected {
                assert_eq!(selected.branch.sources.len(),1);
                if kind==SessionKind::Claude {
                    let event=selected.next(&AtomicBool::new(false)).unwrap().unwrap();
                    let mut complete=Vec::new();event.body.write(&selected.branch,&AtomicBool::new(false),&mut complete,false).unwrap();
                    assert_eq!(String::from_utf8(complete).unwrap(),text);
                }
                // Codex's item telemetry is available through details, while context export keeps
                // its existing response_item scope. Reconciliation must not expand that scope.
                assert!(selected.next(&AtomicBool::new(false)).unwrap().is_none());
            }
            let canceled=AtomicBool::new(true);assert_eq!(matching_display_sources(&branch,&canceled).unwrap_err(),"read_canceled");
            let foreign=Branch {scope:"other".into(),..branch};assert!(matching_display_sources(&foreign,&AtomicBool::new(false)).unwrap_err().starts_with("detail_unavailable:"));
        }}
    }
    #[test]
    fn complete_tool_json_and_execution_state_must_match_beyond_the_preview() {
        for mutation in ["none","late-number","late-text","state"] {
            let temp=super::super::tests::Temp::new();let long="Full original tool content 雪😀\n".repeat(20_000);
            let item=json!({"id":"write","type":"mcpToolCall","server":"native","tool":"write","status":"completed","arguments":{"content":long,"lateNumber":719},"result":{"content":[{"type":"text","text":"Completed original"}]}});
            let mut native=item.clone();match mutation {"late-number"=>native["arguments"]["lateNumber"]=json!(720),"late-text"=>native["arguments"]["content"]=json!(long+"late conflict"),"state"=>native["status"]=json!("failed"),_=>{}}
            let protocol=json!({"method":"item/completed","params":{"threadId":"thread","item":item}}).to_string()+"\n";
            let native=json!({"type":"event_msg","payload":{"type":"item_completed","item":native}}).to_string()+"\n";let branch=indexed(&temp,SessionKind::Codex,&protocol,&native);
            assert_eq!(matching_display_sources(&branch,&AtomicBool::new(false)).unwrap().contains(&branch.sources[1].id),mutation=="none","{mutation}");
        }
    }

    #[test]
    fn full_context_refuses_unknown_records_unconfirmed_delivery_and_unmatched_unique_rows() {
        let temp=super::super::tests::Temp::new();let native=json!({"type":"assistant","uuid":"answer","message":{"content":[{"type":"text","text":"Native answer"}]}}).to_string()+"\n";
        let branch=indexed(&temp,SessionKind::Claude,"{\"type\":\"result\",\"subtype\":\"success\"}\n",&native);
        assert_eq!(native_context(&branch,&AtomicBool::new(false)).unwrap().sources.len(),1);
        assert_eq!(native_context(&branch,&AtomicBool::new(true)).err().unwrap(),"read_canceled");
        let temp=super::super::tests::Temp::new();let branch=indexed(&temp,SessionKind::Claude,"{\"type\":\"future_conversation_message\",\"text\":\"Unique original\"}\n",&native);
        assert!(native_context(&branch,&AtomicBool::new(false)).err().unwrap().starts_with("index_incomplete:"));
        let temp=super::super::tests::Temp::new();let protocol=json!({"type":"assistant","uuid":"unique","message":{"content":[{"type":"text","text":"Unique original"}]}}).to_string()+"\n";
        let branch=indexed(&temp,SessionKind::Claude,&protocol,&native);
        assert!(native_context(&branch,&AtomicBool::new(false)).err().unwrap().starts_with("index_incomplete:"));
        let temp=super::super::tests::Temp::new();let archive=super::super::live::Archive::at(&temp.0).unwrap();let cancel=AtomicBool::new(false);
        let mut journal=super::super::live::Journal::new(archive,"scope",7).unwrap();
        let frame=journal.append("dispatch:input",&json!({"type":"dispatch","id":"input","text":"Unconfirmed original","images":[]}),&cancel).unwrap();
        let store=super::super::store::Store::at(&temp.0).unwrap();let protocol=super::super::protocol_source::register(&store,SessionKind::Claude,&frame.reference,"scope",7,"thread").unwrap();store.scan(SessionKind::Claude,&protocol,&cancel,false).unwrap();
        let path=temp.0.join("native.jsonl");std::fs::write(&path,&native).unwrap();let native=store.register(SessionKind::Claude,super::super::source::Segment {path,native_id:"thread".into(),end_byte:None,end_ordinal:None}).unwrap();store.scan(SessionKind::Claude,&native,&cancel,false).unwrap();
        let branch=Branch {store,sources:vec![native,protocol],generation:"branch".into(),kind:SessionKind::Claude,native_id:"thread".into(),error:None,scope:"scope".into(),parent:None};
        assert!(native_context(&branch,&cancel).err().unwrap().contains("pipe delivery"));
    }

    #[test]
    fn outgoing_images_reconcile_decoded_originals_and_preserve_image_order_and_boundaries() {
        use base64::Engine;
        for mutation in ["none","order","boundary","late"] {
            let temp=super::super::tests::Temp::new();let cancel=AtomicBool::new(false);let archive=super::super::live::Archive::at(&temp.0).unwrap();
            let mut journal=super::super::live::Journal::new(archive,"scope",7).unwrap();let data=[vec![719u32 as u8;40_000],b"BC".to_vec()];
            let images=data.iter().map(|bytes|json!({"mimeType":"image/png","data":base64::engine::general_purpose::STANDARD.encode(bytes)})).collect::<Vec<_>>();
            journal.append("dispatch:input",&json!({"type":"dispatch","id":"input","text":"Complete prompt","images":images}),&cancel).unwrap();
            journal.append("identity:input",&json!({"type":"identity","id":"input","nativeMessageId":"native-input","turnId":"turn"}),&cancel).unwrap();
            let frame=journal.append("delivery:input",&json!({"type":"delivery","id":"input","state":"pipe_written"}),&cancel).unwrap();
            let store=super::super::store::Store::at(&temp.0).unwrap();let protocol=super::super::protocol_source::register(&store,SessionKind::Codex,&frame.reference,"scope",7,"thread").unwrap();store.scan(SessionKind::Codex,&protocol,&cancel,false).unwrap();
            let mut originals=data.to_vec();match mutation {
                "order"=>originals.reverse(),"boundary"=>{originals[0].push(b'B');originals[1].remove(0);},
                "late"=>originals[0][39_999]=b'X',_=>{},
            }
            let mut content=vec![json!({"type":"text","text":"Complete prompt"})];for bytes in originals {
                content.push(json!({"type":"image","imageUrl":format!("data:image/png;base64,{}",base64::engine::general_purpose::STANDARD.encode(bytes))}));
            }
            let raw=json!({"type":"event_msg","payload":{"type":"item_completed","item":{"type":"user_message","id":"native-input","content":content}}}).to_string()+"\n";
            let path=temp.0.join("native.jsonl");std::fs::write(&path,raw).unwrap();let native=store.register(SessionKind::Codex,super::super::source::Segment {path,native_id:"thread".into(),end_byte:None,end_ordinal:None}).unwrap();store.scan(SessionKind::Codex,&native,&cancel,false).unwrap();
            let branch=Branch {store,sources:vec![native,protocol],generation:"branch".into(),kind:SessionKind::Codex,native_id:"thread".into(),error:None,scope:"scope".into(),parent:None};
            assert_eq!(native_context(&branch,&cancel).is_ok(),mutation=="none","{mutation}");
            let conn=branch.store.connect().unwrap();assert_eq!(conn.query_row("SELECT coalesce(sum(reconciled),0) FROM spool_entries",[],|row|row.get::<_,u64>(0)).unwrap(),0);
        }
    }
}

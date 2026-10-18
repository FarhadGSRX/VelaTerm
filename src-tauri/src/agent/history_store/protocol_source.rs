//! Protocol originals use the same verified field reads as native files. Only explicit provider ids
//! associate a display row with an original; text, timestamps and relative display positions do not.

use std::collections::HashMap;
use std::io::Read;
use rusqlite::{params,Connection,OptionalExtension};
use serde_json::{json,Value};

use crate::models::SessionKind;
use super::adapter;
use super::limits::PREVIEW_BYTES;
use super::projection::Record;
use super::store::{digest,sql,RegisteredSource,Store};

pub fn register(store:&Store,kind:SessionKind,reference:&str,scope:&str,epoch:u64,native:&str) -> Result<RegisteredSource,String> {
    if uuid::Uuid::parse_str(reference).is_err() || scope.is_empty() || scope.len()>200 || native.is_empty() || native.len()>200 {
        return Err("detail_unavailable: protocol source identity is invalid".into());
    }
    let path=store.path.parent().unwrap().parent().unwrap().join("chat-spool").join(format!("{reference}.jsonl"));
    let ownership=super::live::read_identity(&path.with_file_name(format!("{reference}.meta.json")))?;
    if ownership["sessionId"]!=scope || ownership["processEpoch"]!=epoch || ownership["reference"]!=reference || ownership["version"]!=1 {
        return Err("detail_unavailable: protocol original ownership does not match its registration".into());
    }
    let direction=ownership["direction"].as_str().unwrap_or("incoming");
    if !matches!(direction,"incoming"|"outgoing") {return Err("detail_unavailable: protocol original direction is invalid".into());}
    let source=store.register(kind,super::source::Segment {path,native_id:native.into(),end_byte:None,end_ordinal:None})?;
    store.connect()?.execute("INSERT INTO protocol_sources VALUES(?1,?2,?3,?4)
        ON CONFLICT(source_id) DO UPDATE SET session_scope=excluded.session_scope,process_epoch=excluded.process_epoch,reference=excluded.reference",
        params![source.id,scope,epoch,reference]).map_err(sql)?;
    set_state(&store.connect()?,&source,"direction",direction)?;Ok(source)
}

pub fn published(store:&Store,kind:SessionKind,scope:&str,native:&str) -> Result<Vec<RegisteredSource>,String> {
    let conn=store.connect()?;let mut statement=conn.prepare("SELECT ref FROM spool_entries WHERE session_scope=?1 AND reconciled=0 ORDER BY rowid LIMIT 129").map_err(sql)?;
    let references=statement.query_map([scope],|row|row.get::<_,String>(0)).map_err(sql)?.collect::<Result<Vec<_>,_>>().map_err(sql)?;
    if references.len()>128 {return Err("unsupported_record: retained protocol source directory requires pagination".into());}
    let mut sources=Vec::new();
    for reference in references {
        if uuid::Uuid::parse_str(&reference).is_err() {continue;}
        let path=store.path.parent().unwrap().parent().unwrap().join("chat-spool").join(format!("{reference}.provider.json"));
        let metadata=match std::fs::symlink_metadata(&path) {Ok(metadata) if metadata.is_file() && metadata.len()<=64*1024=>metadata,
            Err(error) if error.kind()==std::io::ErrorKind::NotFound=>continue,_=>return Err("detail_unavailable: protocol identity is unavailable".into())};
        let mut bytes=Vec::with_capacity(metadata.len() as usize);
        std::fs::File::open(&path).map_err(|_|"detail_unavailable: protocol identity is unavailable")?.take(64*1024+1).read_to_end(&mut bytes).map_err(|_|"detail_unavailable: protocol identity cannot be read")?;
        if bytes.len()>64*1024 {return Err("unsupported_record: protocol identity exceeds its byte budget".into());}
        let identity:Value=serde_json::from_slice(&bytes).map_err(|_|"detail_unavailable: protocol identity is invalid")?;
        if identity["version"]!=1 || identity["reference"].as_str()!=Some(&reference) || identity["sessionId"].as_str()!=Some(scope)
            || identity["provider"].as_str()!=Some(kind.as_str()) {return Err("detail_unavailable: protocol identity does not match its authorized scope".into());}
        let Some(id)=identity["nativeId"].as_str() else {return Err("detail_unavailable: protocol native identity is missing".into());};
        if id!=native {continue;}
        let epoch=identity["processEpoch"].as_u64().ok_or("detail_unavailable: protocol process identity is missing")?;
        sources.push(register(store,kind,&reference,scope,epoch,id)?);
    }
    let mut ordered=Vec::with_capacity(sources.len());
    for source in sources {let epoch=conn.query_row("SELECT process_epoch FROM protocol_sources WHERE source_id=?1",[&source.id],|row|row.get::<_,u64>(0)).map_err(sql)?;ordered.push((epoch,source));}
    ordered.sort_by_key(|(epoch,_)|*epoch);let sources=ordered.into_iter().map(|(_,source)|source).collect();
    Ok(sources)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    fn indexed(temp:&super::super::tests::Temp,kind:SessionKind,raw:&str) -> super::super::api::Branch {
        let archive=super::super::live::Archive::at(&temp.0).unwrap();
        let mut receiver=super::super::live::Receiver::new(raw.as_bytes(),archive,"scope",7);let cancel=AtomicBool::new(false);let mut frame=None;
        while let Some((next,_))=receiver.next(&cancel,&mut |_|Ok(()),&mut |_|panic!("Storage is available")).unwrap() {frame=Some(next);}
        let frame=frame.unwrap();let store=Store::at(&temp.0).unwrap();
        let source=register(&store,kind,&frame.reference,"scope",7,"thread").unwrap();
        store.scan(kind,&source,&cancel,false).unwrap();
        super::super::api::Branch {store,sources:vec![source],generation:"branch".into(),kind,native_id:"thread".into(),error:None,scope:"scope".into(),parent:None}
    }

    #[test]
    fn outgoing_original_is_indexed_only_after_pipe_delivery_and_cannot_be_forged_by_stdout() {
        let temp=super::super::tests::Temp::new();let archive=super::super::live::Archive::at(&temp.0).unwrap();let cancel=AtomicBool::new(false);
        let mut journal=super::super::live::Journal::new(archive.clone(),"scope",7).unwrap();let text="Complete outgoing original 雪😀\n".repeat(5000);
        let proposal=json!({"type":"proposal","id":"msg-1","text":text,"images":[]});
        let dispatch=json!({"type":"dispatch","id":"msg-1","text":text,"images":[],"at":719});
        journal.append("proposal:msg-1",&proposal,&cancel).unwrap();let frame=journal.append("dispatch:msg-1",&dispatch,&cancel).unwrap();
        std::fs::write(temp.0.join("chat-index"),"Derived index unavailable").unwrap();
        archive.bind_outgoing(&frame,SessionKind::Codex,"scope",7,"thread").unwrap();
        assert!(archive.publish(&frame,SessionKind::Codex,"scope",7,"thread").unwrap_err().starts_with("detail_unavailable:"));
        std::fs::remove_file(temp.0.join("chat-index")).unwrap();
        let store=Store::at(&temp.0).unwrap();let source=register(&store,SessionKind::Codex,&frame.reference,"scope",7,"thread").unwrap();
        store.scan(SessionKind::Codex,&source,&cancel,false).unwrap();
        let branch=super::super::api::Branch {store:store.clone(),sources:vec![source],generation:"branch".into(),kind:SessionKind::Codex,native_id:"thread".into(),
            error:None,scope:"scope".into(),parent:None};assert!(branch.page(None,None,None).unwrap()["rows"].as_array().unwrap().is_empty());
        let delivered=json!({"type":"delivery","id":"msg-1","state":"pipe_written"});let frame=journal.append("delivery:msg-1",&delivered,&cancel).unwrap();
        let source=register(&store,SessionKind::Codex,&frame.reference,"scope",7,"thread").unwrap();store.scan(SessionKind::Codex,&source,&cancel,false).unwrap();
        let branch=super::super::api::Branch {sources:vec![source],..branch};let page=branch.page(None,None,None).unwrap();assert_eq!(page["rows"].as_array().unwrap().len(),1);
        let row=&page["rows"][0];assert_eq!(row["at"],719);assert_eq!(row["bodyAvailable"],true);let id=row["id"].as_str().unwrap();
        assert_eq!(row_id(&store,"scope",7,"msg-1").unwrap().as_deref(),Some(id));
        let manifest=branch.manifest(id,0).unwrap();let reference=manifest["fields"][0]["detailRef"].as_str().unwrap();let mut original=Vec::new();
        branch.write_detail(reference,None,&mut original,&cancel).unwrap();assert_eq!(String::from_utf8(original).unwrap(),text);
        assert!(register(&store,SessionKind::Codex,&frame.reference,"foreign",7,"thread").unwrap_err().starts_with("detail_unavailable:"));
        let other=super::super::tests::Temp::new();let forged=format!("{dispatch}\n{delivered}\n");
        assert!(indexed(&other,SessionKind::Codex,&forged).page(None,None,None).unwrap()["rows"].as_array().unwrap().is_empty());
    }

    #[test]
    fn codex_protocol_details_preserve_map_array_replacements_and_late_scalars() {
        let temp=super::super::tests::Temp::new();let diff="雪😀 exact original\n".repeat(90_000);
        let frames=[
            json!({"method":"item/started","params":{"threadId":"thread","item":{"id":"change","type":"fileChange","status":"inProgress","changes":{"old.txt":{"type":"update","unified_diff":"Old diff"}}}}}),
            json!({"method":"item/completed","params":{"threadId":"thread","item":{"id":"change","type":"fileChange","status":"failed","changes":[{"path":"first.json","kind":{"type":"update"},"unified_diff":diff},{"path":"second.txt","kind":{"type":"move"},"content":"Original second file","lateNumber":719,"lateFlag":true}]}}}),
            json!({"method":"turn/completed","params":{"threadId":"thread"}}),
        ];
        let raw=frames.iter().map(|frame|format!("{frame}\n")).collect::<String>();let branch=indexed(&temp,SessionKind::Codex,&raw);
        let page=branch.page(None,None,None).unwrap();assert_eq!(page["rows"].as_array().unwrap().len(),1);assert_eq!(page["coverage"],"complete");
        let row=&page["rows"][0];assert_eq!(row["status"],"failed");assert!(row["input"].to_string().len()<1024);
        let id=row["id"].as_str().unwrap();assert_eq!(row_id(&branch.store,"scope",7,"change").unwrap().as_deref(),Some(id));assert_eq!(row_id(&branch.store,"other",7,"change").unwrap(),None);
        let manifest=branch.manifest(id,0).unwrap();assert_eq!(manifest["files"].as_array().unwrap().len(),2);assert_eq!(manifest["files"][0]["path"],"first.json");assert_eq!(manifest["files"][1]["changeType"],"move");
        let reference=manifest["files"][0]["detailRef"].as_str().unwrap();let mut result=String::new();let mut sequence=0;
        loop {let chunk=branch.detail(reference,None,sequence).unwrap();assert!(chunk.to_string().len()<super::super::limits::DETAIL_BYTES);result.push_str(chunk["text"].as_str().unwrap());if chunk["eof"]==true {break;}sequence+=1;}
        assert_eq!(result,diff);
        let fields=manifest["fields"].as_array().unwrap();let container=fields.iter().find(|field|field["path"]=="/params/item/changes").unwrap()["detailRef"].as_str().unwrap();
        result.clear();sequence=0;loop {let chunk=branch.detail(container,None,sequence).unwrap();result.push_str(chunk["text"].as_str().unwrap());if chunk["eof"]==true {break;}sequence+=1;}
        let original:Value=serde_json::from_str(&result).unwrap();assert_eq!(original[1]["lateNumber"],719);assert_eq!(original[1]["lateFlag"],true);
        assert_eq!(std::fs::read_to_string(&branch.sources[0].segment.path).unwrap(),raw);
    }

    #[test]
    fn append_pieces_remain_individually_readable_until_a_complete_replacement() {
        let temp=super::super::tests::Temp::new();let text="雪😀 appended piece ".repeat(500);
        let mut raw=String::new();for _ in 0..70 {raw.push_str(&format!("{}\n",json!({"method":"item/agentMessage/delta","params":{"threadId":"thread","itemId":"answer","delta":text}})));}
        let mut branch=indexed(&temp,SessionKind::Codex,&raw);let row=branch.page(None,None,None).unwrap()["rows"][0].clone();let id=row["id"].as_str().unwrap();
        assert_eq!(row["text"].as_str().unwrap().len(),PREVIEW_BYTES-1);assert_eq!(row["bodyAvailable"],true);
        let first=branch.manifest(id,0).unwrap();assert_eq!(first["fields"].as_array().unwrap().len(),50);assert_eq!(first["nextCursor"],50);
        let second=branch.manifest(id,50).unwrap();assert_eq!(second["fields"].as_array().unwrap().len(),20);assert_eq!(second["nextCursor"],Value::Null);
        for field in first["fields"].as_array().unwrap().iter().chain(second["fields"].as_array().unwrap()) {
            let chunk=branch.detail(field["detailRef"].as_str().unwrap(),None,0).unwrap();assert_eq!(chunk["text"],text);
        }
        let path=branch.sources[0].segment.path.clone();let final_text="Final complete original 😀 ".repeat(3000);
        let mut file=std::fs::OpenOptions::new().append(true).open(&path).unwrap();use std::io::Write;
        writeln!(file,"{}",json!({"method":"item/completed","params":{"threadId":"thread","item":{"id":"answer","type":"agentMessage","text":final_text}}})).unwrap();drop(file);
        let source=register(&branch.store,SessionKind::Codex,branch.store.connect().unwrap().query_row("SELECT reference FROM protocol_sources",[],|r|r.get::<_,String>(0)).unwrap().as_str(),"scope",7,"thread").unwrap();
        branch.store.scan(SessionKind::Codex,&source,&AtomicBool::new(false),false).unwrap();branch.sources=vec![source];
        let row=branch.page(None,None,None).unwrap()["rows"][0].clone();assert_eq!(row["id"],id);assert_eq!(row["streaming"],false);assert_eq!(row["text"],adapter::preview(&final_text,PREVIEW_BYTES));
        let fields=branch.manifest(id,0).unwrap()["fields"].clone();assert_eq!(fields.as_array().unwrap().len(),1);assert_eq!(fields[0]["sizeBytes"],final_text.len());
        assert_eq!(branch.store.connect().unwrap().query_row("SELECT count(*) FROM append_fields",[],|r|r.get::<_,u64>(0)).unwrap(),0);
    }

    #[test]
    fn child_notifications_are_retained_without_becoming_root_rows() {
        let temp=super::super::tests::Temp::new();let raw=format!("{}\n",json!({"method":"item/completed","params":{"threadId":"child","item":{"id":"answer","type":"agentMessage","text":"Private child body"}}}));
        let branch=indexed(&temp,SessionKind::Codex,&raw);assert_eq!(branch.page(None,None,None).unwrap()["rows"],json!([]));
        assert_eq!(std::fs::read_to_string(&branch.sources[0].segment.path).unwrap(),raw);
    }

    #[test]
    fn outgoing_user_identity_handles_both_delivery_orders_without_text_matching_or_losing_originals() {
        for identity_first in [true,false] {
            let temp=super::super::tests::Temp::new();let cancel=AtomicBool::new(false);
            let archive=super::super::live::Archive::at(&temp.0).unwrap();let store=Store::at(&temp.0).unwrap();
            let mut journal=super::super::live::Journal::new(archive.clone(),"scope",7).unwrap();
            let text="Identical complete prompt 雪😀\n".repeat(3000);let mut last=None;let mut provisional=None;
            for (client,native) in [("first","native-one"),("second","native-two")] {
                let dispatch=json!({"type":"dispatch","id":client,"text":text,"images":[],"at":719});
                journal.append(&format!("dispatch:{client}"),&dispatch,&cancel).unwrap();
                let identity=json!({"type":"identity","id":client,"nativeMessageId":native,"turnId":"same-turn"});
                let delivery=json!({"type":"delivery","id":client,"state":"pipe_written"});
                for value in if identity_first {[identity,delivery]} else {[delivery,identity]} {
                    let frame=journal.append(&format!("{}:{client}",value["type"].as_str().unwrap()),&value,&cancel).unwrap();
                    archive.bind_outgoing(&frame,SessionKind::Codex,"scope",7,"thread").unwrap();
                    let source=register(&store,SessionKind::Codex,&frame.reference,"scope",7,"thread").unwrap();store.scan(SessionKind::Codex,&source,&cancel,false).unwrap();
                    if !identity_first && client=="first" && value["type"]=="delivery" {provisional=row_id(&store,"scope",7,client).unwrap();}
                    last=Some(source);
                }
            }
            let source=last.unwrap();let path=source.segment.path.clone();let before=std::fs::read(&path).unwrap();
            let mut branch=super::super::api::Branch {store:store.clone(),sources:vec![source],generation:"branch".into(),kind:SessionKind::Codex,
                native_id:"thread".into(),error:None,scope:"scope".into(),parent:None};
            let page=branch.page(None,None,None).unwrap();assert_eq!(page["rows"].as_array().unwrap().len(),2);
            let first=page["rows"][0]["id"].as_str().unwrap().to_owned();let second=page["rows"][1]["id"].as_str().unwrap();assert_ne!(first,second);
            assert_eq!(row_id(&store,"scope",7,"first").unwrap().as_deref(),Some(first.as_str()));assert!(row_id(&store,"other",7,"first").unwrap().is_none());
            if let Some(provisional)=provisional {assert_eq!(branch.manifest(&provisional,0).unwrap()["row"]["id"],first);}
            let manifest=branch.manifest(&first,0).unwrap();let reference=manifest["fields"][0]["detailRef"].as_str().unwrap();let mut restored=Vec::new();
            branch.write_detail(reference,None,&mut restored,&cancel).unwrap();assert_eq!(String::from_utf8(restored).unwrap(),text);
            let incoming=format!("{}\n",json!({"method":"item/completed","params":{"threadId":"thread","turnId":"same-turn","item":{"type":"userMessage","id":"native-one","clientId":"first","content":[{"type":"text","text":text}]}}}));
            let received=indexed(&temp,SessionKind::Codex,&incoming);branch.sources.extend(received.sources);
            let conn=store.connect().unwrap();for (sequence,source) in branch.sources.iter().enumerate() {conn.execute("INSERT OR REPLACE INTO segments VALUES('scope',?1,?2,?3,NULL,NULL)",
                params![sequence as u64,source.id,source.generation]).unwrap();}drop(conn);
            assert_eq!(branch.page(None,None,None).unwrap()["rows"].as_array().unwrap().len(),2);
            assert_eq!(std::fs::read(&path).unwrap(),before);
            let conn=store.connect().unwrap();assert_eq!(conn.query_row("SELECT coalesce(sum(reconciled),0) FROM spool_entries",[],|row|row.get::<_,u64>(0)).unwrap(),0);
        }
    }

    #[test]
    fn native_user_items_keep_original_body_and_never_merge_anonymous_repeated_prompts() {
        let temp=super::super::tests::Temp::new();let text="Complete user original 😀\n".repeat(4000);
        let raw=[json!({"type":"event_msg","payload":{"type":"item_completed","item":{"type":"user_message","id":"native-one","content":[{"type":"text","text":text}]}}}),
            json!({"type":"event_msg","payload":{"type":"item_completed","item":{"type":"user_message","id":"native-two","content":[{"type":"text","text":text}]}}})]
            .iter().map(|value|format!("{value}\n")).collect::<String>();
        let branch=super::super::tests::indexed(&temp,&raw);let page=branch.page(None,None,None).unwrap();assert_eq!(page["rows"].as_array().unwrap().len(),2);
        assert_ne!(page["rows"][0]["id"],page["rows"][1]["id"]);assert_eq!(page["rows"][0]["bodyAvailable"],true);
        let manifest=branch.manifest(page["rows"][0]["id"].as_str().unwrap(),0).unwrap();let mut restored=Vec::new();
        branch.write_detail(manifest["fields"][0]["detailRef"].as_str().unwrap(),None,&mut restored,&AtomicBool::new(false)).unwrap();assert_eq!(String::from_utf8(restored).unwrap(),text);
        assert_eq!(std::fs::read_to_string(&branch.sources[0].segment.path).unwrap(),raw);
    }

    #[test]
    fn native_position_and_unique_protocol_revision_are_selected_by_explicit_item_identity() {
        let temp=super::super::tests::Temp::new();let current="Latest original diff 😀\n".repeat(4000);
        let raw=format!("{}\n",json!({"method":"item/completed","params":{"threadId":"thread","item":{"id":"change","type":"fileChange","status":"failed","changes":[{"path":"file.txt","unified_diff":current}]}}}));
        let mut branch=indexed(&temp,SessionKind::Codex,&raw);
        let native_raw=[json!({"type":"event_msg","payload":{"type":"item_completed","item":{"id":"change","type":"file_change","status":"completed","changes":[{"path":"file.txt","unified_diff":"Older native revision"}]}}}),
            json!({"type":"event_msg","payload":{"type":"item_completed","item":{"id":"later","type":"agent_message","content":"Later answer"}}})]
            .iter().map(|row|format!("{row}\n")).collect::<String>();
        let path=temp.0.join("native.jsonl");std::fs::write(&path,&native_raw).unwrap();
        let source=branch.store.register(SessionKind::Codex,super::super::source::Segment {path:path.clone(),native_id:"thread".into(),end_byte:None,end_ordinal:None}).unwrap();
        branch.store.scan(SessionKind::Codex,&source,&AtomicBool::new(false),false).unwrap();branch.sources.insert(0,source);
        let conn=branch.store.connect().unwrap();for (sequence,source) in branch.sources.iter().enumerate() {conn.execute("INSERT INTO segments VALUES('scope',?1,?2,?3,NULL,NULL)",params![sequence as u64,source.id,source.generation]).unwrap();}
        let page=branch.page(None,None,None).unwrap();assert_eq!(page["rows"].as_array().unwrap().len(),2);assert_eq!(page["rows"][0]["status"],"failed");assert_eq!(page["rows"][1]["text"],"Later answer");
        let manifest=branch.manifest(page["rows"][0]["id"].as_str().unwrap(),0).unwrap();let reference=manifest["files"][0]["detailRef"].as_str().unwrap();
        let chunk=branch.detail(reference,None,0).unwrap();assert!(current.starts_with(chunk["text"].as_str().unwrap()));assert!(chunk["text"].as_str().unwrap().len()<=super::super::limits::CHECKPOINT_BYTES+4);
        assert_eq!(std::fs::read_to_string(path).unwrap(),native_raw);assert_eq!(std::fs::read_to_string(&branch.sources[1].segment.path).unwrap(),raw);
        // Original retention is independent of selecting a single row for display. Matching the id
        // alone never proves the native file contains the unique original bytes.
        assert_eq!(conn.query_row("SELECT reconciled FROM spool_entries",[],|r|r.get::<_,u64>(0)).unwrap(),0);
        assert!(super::super::event_stream::Cursor::new(branch,&std::sync::atomic::AtomicBool::new(false)).err().unwrap().starts_with("index_incomplete:"));
    }

    #[test]
    fn claude_streamed_rows_are_replaced_once_when_a_scan_replays_completed_frames() {
        let line=|value:Value|format!("{value}\n");
        let head=[json!({"type":"stream_event","event":{"type":"message_start","message":{"id":"message"}}}),
            json!({"type":"stream_event","event":{"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":"Thought"}}})]
            .into_iter().map(line).collect::<String>();
        let middle=[json!({"type":"assistant","uuid":"thinking-native","message":{"id":"message","content":[{"type":"thinking","thinking":"Thought"}]}}),
            json!({"type":"stream_event","event":{"type":"content_block_stop","index":0}}),
            json!({"type":"stream_event","event":{"type":"content_block_start","index":1,"content_block":{"type":"text","text":""}}}),
            json!({"type":"stream_event","event":{"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"done"}}})]
            .into_iter().map(line).collect::<String>();
        let tail=[json!({"type":"assistant","uuid":"text-native","message":{"id":"message","content":[{"type":"text","text":"done"}]}}),
            json!({"type":"stream_event","event":{"type":"content_block_stop","index":1}}),
            json!({"type":"stream_event","event":{"type":"message_stop"}})].into_iter().map(line).collect::<String>();
        // The message is still streaming when the first scan stops.
        let temp=super::super::tests::Temp::new();let mut branch=indexed(&temp,SessionKind::Claude,&format!("{head}{middle}"));
        let source=branch.sources[0].clone();let reference=branch.store.connect().unwrap().query_row("SELECT reference FROM protocol_sources",[],|row|row.get::<_,String>(0)).unwrap();
        use std::io::Write;std::fs::OpenOptions::new().append(true).open(&source.segment.path).unwrap().write_all(tail.as_bytes()).unwrap();
        let updated=register(&branch.store,SessionKind::Claude,&reference,"scope",7,"thread").unwrap();
        // A scan scheduled from an earlier registration starts at that watermark and replays the
        // completed thinking frame before reaching the text block's completion.
        let stale=RegisteredSource {indexed:head.len() as u64,..updated.clone()};
        branch.store.scan(SessionKind::Claude,&stale,&AtomicBool::new(false),false).unwrap();
        branch.store.scan(SessionKind::Claude,&updated,&AtomicBool::new(false),false).unwrap();branch.sources=vec![updated];
        let rows=branch.page(None,None,None).unwrap()["rows"].as_array().unwrap().iter()
            .map(|row|(row["id"].as_str().unwrap().starts_with("stream:"),row["text"].as_str().unwrap_or_default().to_owned())).collect::<Vec<_>>();
        assert_eq!(rows,vec![(false,"Thought".to_owned()),(false,"done".to_owned())]);
    }

    #[test]
    fn claude_streamed_blocks_keep_original_pieces_and_copied_row_identity_after_completion() {
        let temp=super::super::tests::Temp::new();let piece="雪😀 original append ".repeat(700);let mut raw=String::new();
        for value in [json!({"type":"stream_event","event":{"type":"message_start","message":{"id":"message"}}}),
            json!({"type":"stream_event","event":{"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":"Thought"}}}),
            json!({"type":"assistant","uuid":"thinking-native","message":{"id":"message","content":[{"type":"thinking","thinking":"Thought"}]}}),
            json!({"type":"stream_event","event":{"type":"content_block_start","index":1,"content_block":{"type":"text","text":""}}})] {
            raw.push_str(&format!("{value}\n"));
        }
        for _ in 0..70 {raw.push_str(&format!("{}\n",json!({"type":"stream_event","event":{"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":piece}}})));}
        let mut branch=indexed(&temp,SessionKind::Claude,&raw);let page=branch.page(None,None,None).unwrap();
        assert_eq!(page["rows"].as_array().unwrap().len(),2);let row=page["rows"][1].clone();let provisional=row["id"].as_str().unwrap().to_owned();
        assert!(provisional.starts_with("stream:"));assert_eq!(row["bodyAvailable"],true);assert_eq!(row["bodySizeBytes"],piece.len()*70);
        assert!(row["text"].as_str().unwrap().len()<=PREVIEW_BYTES);let mut from=0;let mut original=String::new();let mut old_ref=None;
        loop {
            let manifest=branch.manifest(&provisional,from).unwrap();
            for field in manifest["fields"].as_array().unwrap() {
                let reference=field["detailRef"].as_str().unwrap();old_ref.get_or_insert_with(||reference.to_owned());
                let mut sequence=0;loop {let chunk=branch.detail(reference,None,sequence).unwrap();original.push_str(chunk["text"].as_str().unwrap());if chunk["eof"]==true {break;}sequence+=1;}
            }
            let Some(next)=manifest["nextCursor"].as_u64() else {break;};from=next;
        }
        assert_eq!(original,piece.repeat(70));
        let source=branch.sources[0].clone();let reference=branch.store.connect().unwrap().query_row("SELECT reference FROM protocol_sources",[],|row|row.get::<_,String>(0)).unwrap();
        use std::io::Write;let mut file=std::fs::OpenOptions::new().append(true).open(&source.segment.path).unwrap();
        let completed="Complete replacement with exact original 😀 ".repeat(2000);
        writeln!(file,"{}",json!({"type":"assistant","uuid":"text-native","message":{"id":"message","content":[{"type":"text","text":completed}]}})).unwrap();drop(file);
        let updated=register(&branch.store,SessionKind::Claude,&reference,"scope",7,"thread").unwrap();branch.store.scan(SessionKind::Claude,&updated,&AtomicBool::new(false),false).unwrap();branch.sources=vec![updated];
        let page=branch.page(None,None,Some(&provisional)).unwrap();assert_eq!(page["rows"].as_array().unwrap().len(),2);
        let canonical=page["rows"][1]["id"].as_str().unwrap();assert_ne!(canonical,provisional);
        assert_eq!(row_id(&branch.store,"scope",7,"message:1").unwrap().as_deref(),Some(canonical));
        let manifest=branch.manifest(&provisional,0).unwrap();assert_eq!(manifest["row"]["id"],canonical);assert_eq!(manifest["fields"].as_array().unwrap().len(),1);
        assert_eq!(manifest["fields"][0]["sizeBytes"],completed.len());assert!(branch.detail(old_ref.as_deref().unwrap(),None,0).is_err());
        let other=super::super::api::Branch {scope:"other".into(),..branch};assert!(other.manifest(&provisional,0).is_err());
    }

    #[test]
    fn claude_complete_blocks_bind_to_their_exact_fields_and_preserve_long_originals() {
        let temp=super::super::tests::Temp::new();let long="Complete Claude text 雪😀\n".repeat(3000);
        let raw=[json!({"type":"stream_event","event":{"type":"message_start","message":{"id":"message"}}}),
            json!({"type":"assistant","uuid":"first-native","message":{"id":"message","role":"assistant","content":[{"type":"thinking","thinking":"Thought"}]}}),
            json!({"type":"assistant","uuid":"second-native","message":{"id":"message","role":"assistant","content":[{"type":"text","text":long},{"type":"tool_use","id":"tool","name":"Write","input":{"content":"Complete input","late":719}}]}})]
            .iter().map(|row|format!("{row}\n")).collect::<String>();
        let branch=indexed(&temp,SessionKind::Claude,&raw);let page=branch.page(None,None,None).unwrap();assert_eq!(page["rows"].as_array().unwrap().len(),3);
        let row=page["rows"].as_array().unwrap().iter().find(|row|row["kind"]=="assistant").unwrap();assert_eq!(row["bodyAvailable"],true);
        assert_eq!(row_id(&branch.store,"scope",7,"message:1").unwrap().as_deref(),row["id"].as_str());assert!(row_id(&branch.store,"scope",8,"message:1").unwrap().is_none());
        let manifest=branch.manifest(row["id"].as_str().unwrap(),0).unwrap();let reference=manifest["fields"][0]["detailRef"].as_str().unwrap();let mut text=String::new();let mut part=0;
        loop {let chunk=branch.detail(reference,None,part).unwrap();text.push_str(chunk["text"].as_str().unwrap());if chunk["eof"]==true {break;}part+=1;}assert_eq!(text,long);
        let tool=row_id(&branch.store,"scope",7,"tool").unwrap().unwrap();assert_eq!(branch.manifest(&tool,0).unwrap()["row"]["name"],"Write");
        assert_eq!(std::fs::read_to_string(&branch.sources[0].segment.path).unwrap(),raw);
    }
}

pub(super) fn state(conn:&Connection,source:&RegisteredSource,key:&str) -> Result<Option<String>,String> {
    conn.query_row("SELECT value FROM protocol_state WHERE source_id=?1 AND generation=?2 AND key=?3",
        params![source.id,source.generation,key],|row|row.get(0)).optional().map_err(sql)
}
pub(super) fn set_state(conn:&Connection,source:&RegisteredSource,key:&str,value:&str) -> Result<(),String> {
    conn.execute("INSERT OR REPLACE INTO protocol_state VALUES(?1,?2,?3,?4)",params![source.id,source.generation,key,value]).map_err(sql)?;Ok(())
}
fn alias(conn:&Connection,source:&RegisteredSource,view:&str,row:&str) -> Result<(),String> {
    conn.execute("INSERT OR REPLACE INTO protocol_aliases VALUES(?1,?2,?3,?4)",params![source.id,source.generation,view,row]).map_err(sql)?;Ok(())
}
pub fn row_id(store:&Store,scope:&str,epoch:u64,view:&str) -> Result<Option<String>,String> {
    store.connect()?.query_row("SELECT a.row_id FROM protocol_aliases a JOIN protocol_sources p ON p.source_id=a.source_id
        JOIN sources s ON s.id=a.source_id AND s.generation=a.generation
        WHERE p.session_scope=?1 AND p.process_epoch=?2 AND a.view_id=?3 LIMIT 1",params![scope,epoch,view],|row|row.get(0)).optional().map_err(sql)
}

pub(super) fn item_id(kind:SessionKind,source:&RegisteredSource,id:&str,prose:bool) -> String {
    if prose {format!("native:{}",digest(format!("{}:{}:{id}:0",kind.as_str(),source.segment.native_id).as_bytes()))}
    else {format!("item:{}",digest(format!("{}:{}:{id}",kind.as_str(),source.segment.native_id).as_bytes()))}
}
fn paths(record:&Record,roots:&[String]) -> Vec<String> {
    record.fields.iter().filter(|field|roots.iter().any(|root|field.path==*root || field.path.starts_with(&(root.clone()+"/"))))
        .map(|field|field.path.clone()).collect()
}
fn replacement(conn:&Connection,source:&RegisteredSource,row:&str,label:&str) -> Result<(),String> {
    // The complete provider value supersedes its append pieces. Old originals remain untouched.
    // An old piece is explicitly unavailable once it no longer belongs to the current row revision.
    conn.execute("DELETE FROM append_fields WHERE ref IN (SELECT ref FROM row_fields WHERE source_id=?1 AND generation=?2 AND row_id=?3 AND label=?4)",
        params![source.id,source.generation,row,label]).map_err(sql)?;
    conn.execute("DELETE FROM row_fields WHERE source_id=?1 AND generation=?2 AND row_id=?3 AND label=?4",params![source.id,source.generation,row,label]).map_err(sql)?;Ok(())
}

pub(super) fn store_record(conn:&Connection,kind:SessionKind,source:&RegisteredSource,record:&Record,refs:&HashMap<String,String>) -> Result<(),String> {
    if state(conn,source,"direction")?.as_deref()==Some("outgoing") {return outgoing(conn,kind,source,record);}
    if kind==SessionKind::Claude {return claude(conn,source,record,refs);}
    // Control facts are still dispatched by the engine. Persist their bounded projection only so
    // recovery can reproduce the protocol's explicit association state without retaining a body.
    conn.execute("INSERT OR REPLACE INTO records(source_id,generation,start,end,event_type,projection,error)
        VALUES(?1,?2,?3,?4,?5,?6,?7)",params![source.id,source.generation,record.start,record.end,
            record.value["method"].as_str().or_else(||record.value["type"].as_str()),record.value.to_string(),record.error]).map_err(sql)?;
    if let Some(error)=&record.error {
        return adapter::write_row(conn,source,record,0,None,&json!({"id":format!("gap:{}:{}:{}",source.id,source.generation,record.start),"kind":"notice","message":error}));
    }
    if kind==SessionKind::Codex {codex(conn,source,record,refs)?;}
    Ok(())
}

fn outgoing(conn:&Connection,kind:SessionKind,source:&RegisteredSource,record:&Record) -> Result<(),String> {
    let value=&record.value;let Some(id)=value["id"].as_str().filter(|id|!id.is_empty() && id.len()<=200) else {
        return Err("unsupported_record: outgoing submission has no bounded identity".into());
    };
    conn.execute("INSERT OR REPLACE INTO records(source_id,generation,start,end,event_type,projection,error) VALUES(?1,?2,?3,?4,?5,?6,?7)",
        params![source.id,source.generation,record.start,record.end,value["type"].as_str(),value.to_string(),record.error]).map_err(sql)?;
    if record.error.is_some() {return Err("unsupported_record: outgoing original could not be projected".into());}
    let delivery_start=if value["type"]=="identity" {
        let native=value["nativeMessageId"].as_str().filter(|id|!id.is_empty() && id.len()<=200)
            .ok_or("unsupported_record: outgoing native user identity is invalid")?;
        let key=format!("client:{id}");
        if state(conn,source,&key)?.is_some_and(|previous|previous!=native) {return Err("index_incomplete: outgoing user identity changed".into());}
        set_state(conn,source,&key,native)?;
        let delivered:Option<u64>=conn.query_row("SELECT start FROM records WHERE source_id=?1 AND generation=?2 AND event_type='delivery'
            AND json_extract(projection,'$.id')=?3 AND json_extract(projection,'$.state')='pipe_written' ORDER BY start DESC LIMIT 1",
            params![source.id,source.generation,id],|row|row.get(0)).optional().map_err(sql)?;
        let Some(delivered)=delivered else {return Ok(());};delivered
    } else if value["type"]=="delivery" && value["state"]=="pipe_written" {record.start} else {return Ok(());};
    let original:Option<(u64,u64,String)>=conn.query_row("SELECT start,end,projection FROM records WHERE source_id=?1 AND generation=?2 AND event_type='dispatch'
        AND json_extract(projection,'$.id')=?3 AND start<?4 AND error IS NULL ORDER BY start DESC LIMIT 1",params![source.id,source.generation,id,delivery_start],
        |row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).optional().map_err(sql)?;
    let (start,end,raw)=original.ok_or("index_incomplete: delivered submission has no persisted dispatch original")?;
    let original:Value=serde_json::from_str(&raw).map_err(|_|"index_incomplete: outgoing dispatch projection is invalid")?;
    let epoch:u64=conn.query_row("SELECT process_epoch FROM protocol_sources WHERE source_id=?1",[&source.id],|row|row.get(0)).map_err(sql)?;
    let provisional=format!("outgoing:{}",digest(format!("{}:{}:{epoch}:{id}",kind.as_str(),source.segment.native_id).as_bytes()));
    let native=state(conn,source,&format!("client:{id}"))?;
    let row_id=native.as_deref().map(|native|item_id(kind,source,native,true)).unwrap_or_else(||provisional.clone());
    if row_id!=provisional {
        for table in ["row_fields","row_parts"] {conn.execute(&format!("DELETE FROM {table} WHERE source_id=?1 AND generation=?2 AND row_id=?3"),
            params![source.id,source.generation,provisional]).map_err(sql)?;}
        conn.execute("DELETE FROM rows WHERE source_id=?1 AND generation=?2 AND id=?3",params![source.id,source.generation,provisional]).map_err(sql)?;
        alias(conn,source,&provisional,&row_id)?;
    }
    let text=original["text"].as_str().ok_or("index_incomplete: outgoing user text projection is unavailable")?;
    let reference:String=conn.query_row("SELECT ref FROM fields WHERE source_id=?1 AND generation=?2 AND record_start=?3 AND path='/text' AND encoding='string'",
        params![source.id,source.generation,start],|row|row.get(0)).map_err(sql)?;
    let size:u64=conn.query_row("SELECT decoded_bytes FROM fields WHERE ref=?1",[&reference],|row|row.get(0)).map_err(sql)?;
    let mut row=json!({"id":row_id,"kind":"user","text":adapter::preview(text,PREVIEW_BYTES),"images":[],"bodySizeBytes":size,
        "bodyAvailable":size>text.len() as u64,"delivery":"pipe_written"});
    if let Some(at)=original["at"].as_u64() {row["at"]=json!(at);}
    let projected=Record {start,end,value:original,fields:Vec::new(),error:None,complete:true};
    adapter::write_row(conn,source,&projected,0,None,&row)?;
    super::attachments::attach(conn,source,&projected,&row_id)?;
    let fields=HashMap::from([("/text".into(),reference)]);
    replacement(conn,source,&row_id,"body")?;adapter::attach(conn,source,&row_id,"body",&["/text".into()],&fields)?;
    alias(conn,source,id,&row_id)?;
    // Delivery originals remain independently retained. A matching native id alone cannot delete them.
    Ok(())
}

fn codex(conn:&Connection,source:&RegisteredSource,record:&Record,refs:&HashMap<String,String>) -> Result<(),String> {
    let value=&record.value;let method=value["method"].as_str().unwrap_or_default();let params=&value["params"];
    // A child thread must never leak into the root history simply because its notification used the
    // same stdout pipe. Its original remains available for a later parent-scoped child adapter.
    if params["threadId"].as_str().is_some_and(|thread|thread!=source.segment.native_id) {return Ok(());}
    if matches!(method,"item/started"|"item/completed") {
        let item=&params["item"];super::children::codex_record(conn,source,record,item)?;let Some(view)=item["id"].as_str() else {return Ok(());};
        let category=item["type"].as_str().unwrap_or_default().replace('_',"").to_ascii_lowercase();
        if category=="usermessage" {return codex_user(conn,source,record,refs,"/params/item");}
        let mut normalized=item.clone();
        if category=="agentmessage" && normalized.get("content").is_none() {normalized["content"]=normalized["text"].clone();}
        let Some(mut row)=crate::agent::chat::history::recorded_codex_item_row(&normalized,value["timestamp"].as_str())
            .and_then(|row|serde_json::to_value(row).ok()) else {return Ok(());};
        let prose=matches!(category.as_str(),"agentmessage"|"reasoning");let id=item_id(SessionKind::Codex,source,view,prose);
        row["id"]=json!(id);row["streaming"]=json!(method=="item/started");
        if row["kind"]=="tool" {
            row["input"]=adapter::short_input(&row["input"]);row["detailAvailable"]=json!(true);
            if let Some(output)=row["output"].as_str() {row["output"]=json!(adapter::preview(output,512));}
        } else if let Some(text)=row["text"].as_str() {row["text"]=json!(adapter::preview(text,PREVIEW_BYTES));}
        let roots:Vec<String>=match category.as_str() {
            "agentmessage"=>vec![if item.get("text").is_some() {"text"} else {"content"}],
            "reasoning"=>["summary_text","summaryText","summary","raw_content","content"].into_iter().find(|key|item.get(*key).is_some()).into_iter().collect(),
            "commandexecution"=>vec!["command","cwd"],"filechange"=>vec!["changes"],
            "mcptoolcall"|"dynamictoolcall"=>vec!["arguments"],"websearch"=>vec!["query"],_=>Vec::new(),
        }.into_iter().map(|root|format!("/params/item/{root}")).collect();
        let mut selected=paths(record,&roots);
        if prose {selected.retain(|path|record.fields.iter().any(|field|field.path==*path && field.encoding=="string"));}
        let long=selected.iter().any(|path|record.fields.iter().any(|field|field.path==*path && field.truncated));
        if prose && long {row["bodyAvailable"]=json!(true);}
        let label=if prose {"body"} else {"input"};
        replacement(conn,source,&id,label)?;
        adapter::write_row(conn,source,record,0,Some(view),&row)?;
        adapter::attach(conn,source,&id,label,&selected,refs)?;
        let outputs:Vec<String>=match category.as_str() {
            "commandexecution"=>vec!["aggregated_output","aggregatedOutput","formatted_output","stdout","stderr"],
            "mcptoolcall"|"dynamictoolcall"=>vec!["result","error"],"websearch"=>vec!["results"],_=>Vec::new(),
        }.into_iter().map(|root|format!("/params/item/{root}")).collect();
        let outputs=paths(record,&outputs);
        if !outputs.is_empty() {replacement(conn,source,&id,"output")?;adapter::attach(conn,source,&id,"output",&outputs,refs)?;}
        adapter::store_files_at(conn,source,&id,record.start,item,refs,"/params/item/changes")?;
        alias(conn,source,view,&id)?;
    } else if matches!(method,"item/agentMessage/delta"|"item/reasoning/textDelta"|"item/reasoning/summaryTextDelta"|"item/commandExecution/outputDelta") {
        let Some(view)=params["itemId"].as_str() else {return Ok(());};
        let Some(field)=record.fields.iter().find(|field|field.path=="/params/delta" && field.encoding=="string") else {return Ok(());};
        let text=params["delta"].as_str().unwrap_or_default();let prose=method!="item/commandExecution/outputDelta";
        let id=item_id(SessionKind::Codex,source,view,prose);
        let old:Option<String>=conn.query_row("SELECT summary FROM rows WHERE source_id=?1 AND generation=?2 AND id=?3",
            params![source.id,source.generation,id],|row|row.get(0)).optional().map_err(sql)?;
        let mut row=old.as_deref().map(serde_json::from_str::<Value>).transpose().map_err(|e|e.to_string())?.unwrap_or_else(||
            if prose {json!({"kind":if method=="item/agentMessage/delta" {"assistant"} else {"reasoning"},"id":id,"text":"","streaming":true})}
            else {json!({"kind":"tool","id":id,"name":"Bash","input":{},"output":"","isError":false,"status":"running","detailAvailable":true})});
        let key=if prose {"text"} else {"output"};let mut preview=row[key].as_str().unwrap_or_default().to_owned();
        preview.push_str(&adapter::preview(text,PREVIEW_BYTES.saturating_sub(preview.len())));row[key]=json!(preview);
        let bytes=row["bodySizeBytes"].as_u64().unwrap_or(0).saturating_add(field.decoded_bytes);row["bodySizeBytes"]=json!(bytes);
        if prose && bytes>PREVIEW_BYTES as u64 {row["bodyAvailable"]=json!(true);}
        adapter::write_row(conn,source,record,0,Some(view),&row)?;
        adapter::attach(conn,source,&id,if prose {"body"} else {"output"},&[field.path.clone()],refs)?;
        if let Some(reference)=refs.get(&field.path) {conn.execute("INSERT OR IGNORE INTO append_fields VALUES(?1)",[reference]).map_err(sql)?;}
        alias(conn,source,view,&id)?;
    }
    Ok(())
}

pub(super) fn codex_user(conn:&Connection,source:&RegisteredSource,record:&Record,refs:&HashMap<String,String>,root:&str) -> Result<(),String> {
    let item=record.value.pointer(root).ok_or("unsupported_record: native user item is missing")?;
    let native=item["id"].as_str().filter(|id|!id.is_empty() && id.len()<=200).ok_or("unsupported_record: native user item has no bounded identity")?;
    let content=item["content"].as_array().ok_or("unsupported_record: native user content is unavailable")?;
    let selected=content.iter().enumerate().filter(|(_,part)|matches!(part["type"].as_str(),Some("text"|"input_text")))
        .map(|(index,_)|format!("{root}/content/{index}/text")).collect::<Vec<_>>();
    let mut text=String::new();let mut size=0u64;let mut parts=0;
    for path in &selected {if let Some(field)=record.fields.iter().find(|field|field.path==*path) {
        if parts>0 {if text.len()<PREVIEW_BYTES {text.push('\n');}size+=1;}parts+=1;
        size+=field.decoded_bytes;
        let part=record.value.pointer(path).and_then(Value::as_str).ok_or("unsupported_record: native user text is invalid")?;
        let remaining=PREVIEW_BYTES.saturating_sub(text.len());text.push_str(&adapter::preview(part,remaining));
    }}
    let id=item_id(SessionKind::Codex,source,native,true);
    let row=json!({"id":id,"kind":"user","text":text,"images":[],"bodySizeBytes":size,"bodyAvailable":size>text.len() as u64});
    adapter::write_row(conn,source,record,0,Some(native),&row)?;
    replacement(conn,source,&id,"body")?;adapter::attach(conn,source,&id,"body",&selected,refs)?;
    alias(conn,source,native,&id)?;
    if let Some(client)=item["clientId"].as_str().filter(|id|!id.is_empty() && id.len()<=200) {
        let previous:Option<String>=conn.query_row("SELECT row_id FROM protocol_aliases WHERE source_id=?1 AND generation=?2 AND view_id=?3",
            params![source.id,source.generation,client],|row|row.get(0)).optional().map_err(sql)?;
        if previous.is_some_and(|previous|previous!=id) {return Err("index_incomplete: the provider returned conflicting client user identities".into());}
        alias(conn,source,client,&id)?;
    }
    Ok(())
}

fn stream_id(source:&RegisteredSource,message:&str,index:u64) -> String {
    format!("stream:{}",digest(format!("claude:{}:{message}:{index}",source.segment.native_id).as_bytes()))
}

fn claude_stream(conn:&Connection,source:&RegisteredSource,record:&Record,refs:&HashMap<String,String>) -> Result<(),String> {
    let event=&record.value["event"];
    match event["type"].as_str() {
        Some("message_start")=> {
            if let Some(message)=event.pointer("/message/id").and_then(Value::as_str) {
                set_state(conn,source,"currentMessage",message)?;
                set_state(conn,source,&format!("blocks:{message}"),"0")?;
            }
        },
        Some("message_stop")=>{set_state(conn,source,"currentMessage","")?;},
        Some("content_block_start"|"content_block_delta"|"content_block_stop")=> {
            let Some(message)=state(conn,source,"currentMessage")?.filter(|message|!message.is_empty()) else {return Ok(());};
            let index=event["index"].as_u64().unwrap_or(0);
            let view=format!("{message}:{index}");let provisional=stream_id(source,&message,index);
            let key=format!("stream:{message}:{index}");
            if event["type"]=="content_block_start" {
                let block=&event["content_block"];let category=block["type"].as_str().unwrap_or("text");
                let mut row=match category {
                    "tool_use"=> {
                        let Some(call)=block["id"].as_str() else {return Ok(());};
                        let id=format!("call:{}",digest(format!("claude:{}:{call}",source.segment.native_id).as_bytes()));
                        alias(conn,source,call,&id)?;
                        json!({"id":id,"kind":"tool","name":block["name"].as_str().unwrap_or("tool"),"input":adapter::short_input(&block["input"]),
                            "status":"running","isError":false,"detailAvailable":true})
                    },
                    "text"|"thinking"=>json!({"id":provisional,"kind":if category=="thinking" {"reasoning"} else {"assistant"},"text":"","streaming":true}),
                    _=>return Ok(()),
                };
                let id=row["id"].as_str().unwrap().to_owned();
                let field=if category=="tool_use" {"input"} else if category=="thinking" {"thinking"} else {"text"};
                let path=format!("/event/content_block/{field}");
                if field!="input" {row["text"]=json!(adapter::preview(block[field].as_str().unwrap_or_default(),PREVIEW_BYTES));}
                let selected=paths(record,&[path]);let size=record.fields.iter().filter(|field|selected.contains(&field.path) && field.encoding=="string")
                    .map(|field|field.decoded_bytes).sum::<u64>();row["bodySizeBytes"]=json!(size);
                if size>PREVIEW_BYTES as u64 && category!="tool_use" {row["bodyAvailable"]=json!(true);}
                adapter::write_row(conn,source,record,0,None,&row)?;
                adapter::attach(conn,source,&id,if category=="tool_use" {"input"} else {"body"},&selected,refs)?;
                alias(conn,source,&view,&id)?;alias(conn,source,&provisional,&id)?;set_state(conn,source,&key,&id)?;
            } else {
                let Some(id)=state(conn,source,&key)? else {return Ok(());};
                let raw:Option<String>=conn.query_row("SELECT summary FROM rows WHERE source_id=?1 AND generation=?2 AND id=?3",
                    params![source.id,source.generation,id],|row|row.get(0)).optional().map_err(sql)?;
                let Some(raw)=raw else {return Ok(());};let mut row:Value=serde_json::from_str(&raw).map_err(|e|e.to_string())?;
                if event["type"]=="content_block_stop" {row["streaming"]=json!(false);}
                else {
                    let field=match event.pointer("/delta/type").and_then(Value::as_str) {
                        Some("text_delta")=>"text",Some("thinking_delta")=>"thinking",Some("input_json_delta")=>"partial_json",_=>return Ok(()),
                    };
                    let path=format!("/event/delta/{field}");
                    let Some(original)=record.fields.iter().find(|original|original.path==path && original.encoding=="string") else {return Ok(());};
                    let size=row["bodySizeBytes"].as_u64().unwrap_or(0).saturating_add(original.decoded_bytes);row["bodySizeBytes"]=json!(size);
                    if row["kind"]!="tool" {
                        let mut preview=row["text"].as_str().unwrap_or_default().to_owned();
                        preview.push_str(&adapter::preview(event["delta"][field].as_str().unwrap_or_default(),PREVIEW_BYTES.saturating_sub(preview.len())));
                        row["text"]=json!(preview);if size>PREVIEW_BYTES as u64 {row["bodyAvailable"]=json!(true);}
                    }
                    adapter::attach(conn,source,&id,if row["kind"]=="tool" {"input"} else {"body"},&[path.clone()],refs)?;
                    if let Some(reference)=refs.get(&path) {conn.execute("INSERT OR IGNORE INTO append_fields VALUES(?1)",[reference]).map_err(sql)?;}
                }
                adapter::write_row(conn,source,record,0,None,&row)?;
            }
        },
        _=>{},
    }
    Ok(())
}

fn complete_stream_alias(conn:&Connection,source:&RegisteredSource,view:&str,provisional:&str,canonical:&str) -> Result<(),String> {
    if provisional!=canonical {
        let position:Option<(String,u64,u64)>=conn.query_row("SELECT order_key,record_start,revision FROM rows WHERE source_id=?1 AND generation=?2 AND id=?3",
            params![source.id,source.generation,provisional],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).optional().map_err(sql)?;
        if let Some((order,start,revision))=position {
            conn.execute("UPDATE rows SET order_key=min(order_key,?4),record_start=min(record_start,?5),revision=max(revision,?6+1)
                WHERE source_id=?1 AND generation=?2 AND id=?3",params![source.id,source.generation,canonical,order,start,revision]).map_err(sql)?;
            // Completion explicitly binds message/block identity to the native uuid. Keep old copied
            // row locators as aliases, while replacement fields come from the full completed value.
            replacement(conn,source,provisional,"body")?;replacement(conn,source,provisional,"input")?;
            conn.execute("DELETE FROM rows WHERE source_id=?1 AND generation=?2 AND id=?3",params![source.id,source.generation,provisional]).map_err(sql)?;
        }
        conn.execute("UPDATE protocol_aliases SET row_id=?4 WHERE source_id=?1 AND generation=?2 AND row_id=?3",
            params![source.id,source.generation,provisional,canonical]).map_err(sql)?;
    }
    alias(conn,source,view,canonical)?;alias(conn,source,provisional,canonical)?;Ok(())
}

fn claude(conn:&Connection,source:&RegisteredSource,record:&Record,refs:&HashMap<String,String>) -> Result<(),String> {
    // The ordinary complete stdout message has the native transcript shape, including its uuid.
    // Root and child scopes are separate; root pages do not recursively materialize child messages.
    let value=&record.value;
    if value["parent_tool_use_id"].as_str().is_some() {
        conn.execute("INSERT OR REPLACE INTO records(source_id,generation,start,end,event_type,projection,error) VALUES(?1,?2,?3,?4,?5,?6,?7)",
            params![source.id,source.generation,record.start,record.end,value["type"].as_str(),value.to_string(),record.error]).map_err(sql)?;return Ok(());
    }
    adapter::store_record(conn,SessionKind::Claude,source,record,refs)?;
    if value["type"]=="stream_event" {claude_stream(conn,source,record,refs)?;}
    if value["type"]=="assistant" {
        let Some(message)=value.pointer("/message/id").and_then(Value::as_str) else {return Ok(());};
        let Some(blocks)=value.pointer("/message/content").and_then(Value::as_array) else {return Ok(());};
        // Each complete frame keeps the block offset it was first given. A scan resumed from an earlier
        // watermark replays frames already counted; adding them again would point later frames at
        // the wrong streamed rows and leave those rows behind as duplicates.
        let counter=format!("blocks:{message}");let frame=format!("frame:{message}:{}",record.start);
        let next=state(conn,source,&counter)?.and_then(|value|value.parse::<usize>().ok()).unwrap_or(0);
        let offset=match state(conn,source,&frame)?.and_then(|value|value.parse::<usize>().ok()) {
            Some(offset)=>offset,
            None=>{set_state(conn,source,&frame,&next.to_string())?;next},
        };
        for (index,block) in blocks.iter().enumerate() {
            if let Some(call)=block["id"].as_str().filter(|_|block["type"]=="tool_use") {
                let id=format!("call:{}",digest(format!("claude:{}:{call}",source.segment.native_id).as_bytes()));
                replacement(conn,source,&id,"input")?;
                let roots=paths(record,&[format!("/message/content/{index}/input")]);adapter::attach(conn,source,&id,"input",&roots,refs)?;
                alias(conn,source,call,&id)?;
            } else {
                let kind=if block["type"]=="thinking" {"reasoning"} else {"assistant"};
                // The field binding gives the exact row even when empty or unsupported blocks were
                // filtered by the semantic adapter. A guessed local event number is never sufficient.
                let path=format!("/message/content/{index}/{}",if kind=="reasoning" {"thinking"} else {"text"});
                if let Some(reference)=refs.get(&path) {
                    let id:Option<String>=conn.query_row("SELECT row_id FROM row_fields WHERE source_id=?1 AND generation=?2 AND ref=?3 LIMIT 1",
                        params![source.id,source.generation,reference],|row|row.get(0)).optional().map_err(sql)?;
                    if let Some(id)=id {
                        let view=format!("{message}:{}",offset+index);let provisional=stream_id(source,message,(offset+index) as u64);
                        complete_stream_alias(conn,source,&view,&provisional,&id)?;
                    }
                }
            }
        }
        set_state(conn,source,&counter,&next.max(offset+blocks.len()).to_string())?;
    }
    Ok(())
}

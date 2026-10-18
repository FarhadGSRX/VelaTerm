//! Verified paired Kiro JSONL history. Metadata turns are streamed separately and must agree with
//! their explicitly referenced assistant bodies before any staged event becomes visible.

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read,Seek,SeekFrom};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool,Ordering};
use rusqlite::{params,Connection,OptionalExtension};
use serde_json::{json,Value};
use sha2::{Digest,Sha256};
use crate::{agent::kiro_store,models::SessionKind};
use super::{adapter,api,limits::*,projection::{Checkpoint,Projector,Record},source::{self,Segment},store::{Store,RegisteredSource,sql,digest}};

#[derive(Clone,Debug)]
pub(super) struct Source {pub(super) metadata:RegisteredSource,database:Option<RegisteredSource>}

pub(super) fn register(store:&Arc<Store>,path:PathBuf,native:&str) -> Result<RegisteredSource,String> {
    register_from(store,path,native,kiro_store::Roots::native()?.database)
}
fn register_from(store:&Arc<Store>,path:PathBuf,native:&str,database:Option<PathBuf>) -> Result<RegisteredSource,String> {
    if !kiro_store::valid_id(native) || path.file_stem().and_then(|stem|stem.to_str())!=Some(native) {return Err("source_changed: Kiro filename and session identities conflict".into());}
    for path in [path.clone(),path.with_extension("json")] {
        if !std::fs::symlink_metadata(path).is_ok_and(|metadata|metadata.is_file()) {return Err("source_missing: Kiro paired sources must be available regular files".into());}
    }
    let metadata=store.register(SessionKind::Kiro,Segment {path:path.with_extension("json"),native_id:native.into(),end_byte:None,end_ordinal:None})
        .map_err(|_|"source_missing: Kiro paired metadata is unavailable")?;
    let database=database.filter(|path|path.is_file()).map(|path|super::kiro_database::register(store,path,native)).transpose()?;
    let mut source=store.register(SessionKind::Kiro,Segment {path,native_id:native.into(),end_byte:None,end_ordinal:None})?;
    let revision=digest(json!([metadata.id,metadata.generation,database.as_ref().map(|source|(&source.id,&source.generation))]).to_string().as_bytes());
    let _gate=store.cache_gate.lock().unwrap();let conn=store.connect()?;
    let previous:String=conn.query_row("SELECT branch_revision FROM sources WHERE id=?1",[&source.id],|row|row.get(0)).map_err(sql)?;
    if !previous.is_empty() && previous!=revision {
        source.generation=uuid::Uuid::new_v4().to_string();source.indexed=0;source.status="partial".into();
        source._pin=super::cache::Pin::new(store,format!("{}:{}",source.id,source.generation));
    }
    conn.execute("UPDATE sources SET generation=?2,indexed_bytes=?3,status=?4,branch_revision=?5,error=NULL WHERE id=?1",
        params![source.id,source.generation,source.indexed,source.status,revision]).map_err(sql)?;
    conn.execute("INSERT INTO generation_access VALUES(?1,?2,?3) ON CONFLICT(source_id,generation) DO UPDATE SET touched=excluded.touched",
        params![source.id,source.generation,super::cache::now()]).map_err(sql)?;
    for (key,cancel) in store.jobs.lock().unwrap().iter() {if key.starts_with(&format!("{}:",source.id)) && key!=&format!("{}:{}",source.id,source.generation) {cancel.store(true,Ordering::Relaxed);}}
    source.kiro_pair=Some(Box::new(Source {metadata,database}));Ok(source)
}

struct Input<'a> {file:File,conn:&'a Connection,source:&'a RegisteredSource,cancel:&'a AtomicBool,offset:u64}
impl Read for Input<'_> {
    fn read(&mut self,bytes:&mut [u8]) -> std::io::Result<usize> {
        if self.cancel.load(Ordering::Relaxed) {return Err(std::io::Error::other("read_canceled"));}
        let request=bytes.len().min(self.source.size.saturating_sub(self.offset) as usize);if request==0 {return Ok(0);}
        let count=self.file.read(&mut bytes[..request])?;
        if count>0 {
            let hash=Sha256::digest(&bytes[..count]).to_vec();
            let old:Option<(u64,Vec<u8>)>=self.conn.query_row("SELECT end,hash FROM source_blocks WHERE source_id=?1 AND generation=?2 AND start=?3",
                params![self.source.id,self.source.generation,self.offset],|row|Ok((row.get(0)?,row.get(1)?))).optional().map_err(std::io::Error::other)?;
            if old.is_some_and(|(end,old)|end==self.offset+count as u64 && old!=hash) {return Err(std::io::Error::other("source_changed: Kiro paired source changed"));}
            self.conn.execute("INSERT OR REPLACE INTO source_blocks VALUES(?1,?2,?3,?4,?5)",params![self.source.id,self.source.generation,self.offset,self.offset+count as u64,hash])
                .map_err(std::io::Error::other)?;self.offset+=count as u64;
        }Ok(count)
    }
}
fn input<'a>(conn:&'a Connection,source:&'a RegisteredSource,cancel:&'a AtomicBool) -> Result<Input<'a>,String> {
    let file=File::open(&source.segment.path).map_err(|_|"source_missing: Kiro paired source is unavailable")?;
    let expected:String=conn.query_row("SELECT identity FROM sources WHERE id=?1 AND generation=?2",params![source.id,source.generation],|row|row.get(0)).map_err(sql)?;
    let metadata=file.metadata().map_err(|_|"source_missing: Kiro paired source is unavailable")?;
    if source::identity(&metadata)!=expected || metadata.len()!=source.size {return Err("source_changed: Kiro paired source was replaced or resized".into());}
    Ok(Input {file,conn,source,cancel,offset:0})
}
fn checkpoint(conn:&Connection,source:&RegisteredSource,piece:Checkpoint) -> Result<(),String> {
    conn.execute("INSERT OR REPLACE INTO checkpoints VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",params![source.id,source.generation,piece.record_start,
        piece.path,piece.raw_start,piece.raw_end,piece.decoded_start,piece.decoded_end,piece.hash,piece.sequence]).map_err(sql)?;Ok(())
}
fn boundary(conn:&Connection,source:&RegisteredSource,modified:&str) -> Result<(),String> {
    let (generation,identity):(String,String)=conn.query_row("SELECT generation,identity FROM sources WHERE id=?1",[&source.id],|row|Ok((row.get(0)?,row.get(1)?))).map_err(sql)?;
    let metadata=std::fs::symlink_metadata(&source.segment.path).map_err(|_|"source_changed: Kiro paired source disappeared")?;
    if generation!=source.generation || !metadata.is_file() || source::identity(&metadata)!=identity || metadata.len()!=source.size
        || format!("{:?}",metadata.modified().ok())!=modified {return Err("source_changed: Kiro paired source changed while reading".into());}Ok(())
}
fn blank(source:&RegisteredSource,record:&Record,cancel:&AtomicBool) -> Result<bool,String> {
    let mut file=File::open(&source.segment.path).map_err(|_|"source_missing: Kiro paired source is unavailable")?;file.seek(SeekFrom::Start(record.start)).map_err(|e|e.to_string())?;
    let mut buffer=vec![0;INPUT_BYTES];let mut bytes=record.end-record.start;
    while bytes>0 {if cancel.load(Ordering::Relaxed) {return Err("read_canceled".into());}
        let request=bytes.min(INPUT_BYTES as u64) as usize;file.read_exact(&mut buffer[..request]).map_err(|_|"source_changed: Kiro source was truncated")?;
        if !buffer[..request].iter().all(u8::is_ascii_whitespace) {return Ok(false);}bytes-=request as u64;
    }Ok(true)
}

fn event(conn:&Connection,source:&RegisteredSource,record:&mut Record) -> Result<(),String> {
    let value=&record.value;
    if value["version"]!="v1" {return Err("unsupported_record: Kiro event version is unsupported".into());}
    let role=match value["kind"].as_str() {Some("Prompt")=>"user",Some("AssistantMessage")=>"assistant",_=>return Err("unsupported_record: Kiro event type is unsupported".into())};
    let id=value["data"]["message_id"].as_str().filter(|id|kiro_store::valid_id(id)).ok_or("unsupported_record: Kiro message identity is invalid")?;
    let timestamp=kiro_store::parse::seconds(&value["data"]["meta"]["timestamp"])?;
    let blocks=value["data"]["content"].as_array().ok_or("unsupported_record: Kiro content blocks are invalid")?;
    let mut preview=String::new();let mut paths=Vec::new();let mut size=0u64;
    for (number,block) in blocks.iter().enumerate() {
        if block["kind"]!="text" {return Err("unsupported_record: Kiro contains unverified non-text content".into());}
        let text=block["data"].as_str().ok_or("unsupported_record: Kiro text block is invalid")?;let path=format!("/data/content/{number}/data");
        let field=record.fields.iter().find(|field|field.path==path && field.encoding=="string").ok_or("unsupported_record: Kiro text range is unavailable")?;
        preview.push_str(&adapter::preview(text,PREVIEW_BYTES.saturating_sub(preview.len())));size=size.saturating_add(field.decoded_bytes);paths.push(path);
    }
    let duplicate:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM records WHERE source_id=?1 AND generation=?2 AND native_id=?3 AND start!=?4)",
        params![source.id,source.generation,id,record.start],|row|row.get(0)).map_err(sql)?;
    if duplicate {return Err("unsupported_record: duplicate Kiro message identity".into());}
    let refs=adapter::store_fields(conn,source,record)?;
    conn.execute("INSERT OR REPLACE INTO records(source_id,generation,start,end,native_id,event_type,projection,active) VALUES(?1,?2,?3,?4,?5,'kiro_message',?6,0)",
        params![source.id,source.generation,record.start,record.end,id,json!({"type":"kiro_message","id":id,"role":role,"text":preview,"timestamp":timestamp,"content":blocks}).to_string()]).map_err(sql)?;
    let stable=format!("native:{}",digest(format!("kiro-file:{}:{id}",source.segment.native_id).as_bytes()));
    let mut row=json!({"id":stable,"kind":role,"text":preview,"streaming":false,"bodySizeBytes":size});
    if size>PREVIEW_BYTES as u64 || record.fields.iter().any(|field|paths.contains(&field.path) && field.truncated) {row["bodyAvailable"]=json!(true);}
    if let Some(at)=crate::agent::chat::history::parsed_at(timestamp.as_deref()) {row["at"]=json!(at);}
    adapter::write_row(conn,source,record,0,None,&row)?;adapter::attach(conn,source,&stable,"body",&paths,&refs)?;
    for path in &paths {if let Some(reference)=refs.get(path) {conn.execute("INSERT OR IGNORE INTO append_fields VALUES(?1)",[reference]).map_err(sql)?;}}
    Ok(())
}

fn references(conn:&Connection,source:&RegisteredSource,start:u64) -> Result<HashMap<String,String>,String> {
    let mut statement=conn.prepare("SELECT path,ref FROM fields WHERE source_id=?1 AND generation=?2 AND record_start=?3 AND encoding='string'").map_err(sql)?;
    let rows=statement.query_map(params![source.id,source.generation,start],|row|Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?))).map_err(sql)?;
    rows.collect::<Result<HashMap<_,_>,_>>().map_err(sql)
}
fn field_hash(conn:&Connection,source:&RegisteredSource,reference:&str,cancel:&AtomicBool) -> Result<Vec<u8>,String> {
    let mut hash=Sha256::new();let mut sequence=0;
    loop {if cancel.load(Ordering::Relaxed) {return Err("read_canceled".into());}
        let (text,_,end,size)=api::read_piece(conn,source,reference,sequence,0)?;hash.update(text.as_bytes());if end>=size {break;}sequence+=1;
    }Ok(hash.finalize().to_vec())
}
fn canonical(conn:&Connection,source:&RegisteredSource,value:&Value,prefix:&str,refs:&HashMap<String,String>,cancel:&AtomicBool) -> Result<String,String> {
    let mut value=value.clone();
    for (path,reference) in refs {
        if let Some(relative)=path.strip_prefix(prefix).filter(|path|path.is_empty() || path.starts_with('/')) {
            if let Some(field)=value.pointer_mut(relative) {if field.is_string() {*field=json!(hex(&field_hash(conn,source,reference,cancel)?));}}
        }
    }Ok(value.to_string())
}
fn hex(bytes:&[u8]) -> String {bytes.iter().map(|byte|format!("{byte:02x}")).collect()}

fn turn(conn:&Connection,source:&RegisteredSource,metadata:&RegisteredSource,record:&Record,cancel:&AtomicBool) -> Result<(),String> {
    let refs=adapter::store_fields(conn,metadata,record)?;
    let ids=record.value["message_ids"].as_array().ok_or("unsupported_record: Kiro turn message identities are invalid")?;let mut assistant=None;
    for id in ids {
        let id=id.as_str().filter(|id|kiro_store::valid_id(id)).ok_or("unsupported_record: Kiro turn message identity is invalid")?;
        let assigned:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM kiro_assigned WHERE id=?1)",[id],|row|row.get(0)).map_err(sql)?;
        if assigned {return Err("unsupported_record: Kiro message belongs to multiple turns".into());}
        conn.execute("INSERT INTO kiro_assigned VALUES(?1)",[id]).map_err(sql)?;
        let event:Option<(u64,String)>=conn.query_row("SELECT start,projection FROM records WHERE source_id=?1 AND generation=?2 AND native_id=?3",
            params![source.id,source.generation,id],|row|Ok((row.get(0)?,row.get(1)?))).optional().map_err(sql)?;
        let (start,value)=event.ok_or("index_incomplete: Kiro turn references a missing message")?;
        let value:Value=serde_json::from_str(&value).map_err(|_|"index_incomplete: Kiro message projection is invalid")?;
        if value["role"]=="assistant" {assistant=Some((start,value));}
    }
    if !record.value["result"].is_null() {
        let result=record.value["result"].get("Ok").ok_or("unsupported_record: Kiro turn result is unsupported")?;
        let (start,assistant)=assistant.ok_or("index_incomplete: Kiro turn assistant reference is unavailable")?;
        let assistant_refs=references(conn,source,start)?;
        if canonical(conn,metadata,&result["content"],"/result/Ok/content",&refs,cancel)?!=canonical(conn,source,&assistant["content"],"/data/content",&assistant_refs,cancel)? {
            return Err("source_changed: Kiro turn result conflicts with its explicitly referenced assistant content".into());
        }
        kiro_store::parse::seconds(&result["meta"]["timestamp"])?;
    }
    kiro_store::parse::iso_optional(&record.value["end_timestamp"])?;Ok(())
}

fn model_context(model:&Value,percent:&Value) -> Result<Value,String> {
    let context=kiro_store::parse::context(model,percent)?;Ok(json!([context.model,context.limit,context.percent]))
}
fn corpus(conn:&Connection,source:&RegisteredSource,cancel:&AtomicBool) -> Result<Vec<u8>,String> {
    let mut statement=conn.prepare("SELECT start,event_type,projection FROM records WHERE source_id=?1 AND generation=?2 AND event_type IN ('kiro_message','kiro_pair') ORDER BY start").map_err(sql)?;
    let mut records=statement.query(params![source.id,source.generation]).map_err(sql)?;let mut corpus=Sha256::new();
    while let Some(row)=records.next().map_err(sql)? {
        let start:u64=row.get(0).map_err(sql)?;let kind:String=row.get(1).map_err(sql)?;let raw:String=row.get(2).map_err(sql)?;
        let value:Value=serde_json::from_str(&raw).map_err(|_|"index_incomplete: Kiro corpus projection is invalid")?;let refs=references(conn,source,start)?;
        let roles=if kind=="kiro_pair" {vec!["user","assistant"]} else {vec![value["role"].as_str().ok_or("index_incomplete: Kiro message role is unavailable")?]};
        for role in roles {
            let timestamp=if role=="user" || kind=="kiro_message" {value["timestamp"].clone()} else {Value::Null};
            corpus.update(json!([role,timestamp]).to_string().as_bytes());let mut body=Sha256::new();
            let paths=if kind=="kiro_pair" {vec![if role=="user" {"/user/content/Prompt/prompt".into()} else {"/assistant/Response/content".into()}]}
                else {let blocks=value["content"].as_array().ok_or("index_incomplete: Kiro content blocks are unavailable")?;
                    (0..blocks.len()).map(|number|format!("/data/content/{number}/data")).collect::<Vec<_>>()};
            for path in paths {let reference=refs.get(&path).ok_or("index_incomplete: Kiro body original is unavailable")?;let mut sequence=0;
                loop {if cancel.load(Ordering::Relaxed) {return Err("read_canceled".into());}
                    let (text,_,end,size)=api::read_piece(conn,source,reference,sequence,0)?;body.update(text.as_bytes());if end>=size {break;}sequence+=1;
                }
            }corpus.update(body.finalize());
        }
    }Ok(corpus.finalize().to_vec())
}

pub(super) fn scan(store:&Store,source:&RegisteredSource,cancel:&AtomicBool) -> Result<(),String> {
    let descriptor=source.kiro_pair.as_ref().ok_or("unsupported_record: Kiro paired descriptor is unavailable")?;let metadata=&descriptor.metadata;let conn=store.connect()?;
    let log_modified:String=conn.query_row("SELECT modified FROM sources WHERE id=?1 AND generation=?2",params![source.id,source.generation],|row|row.get(0)).map_err(sql)?;
    let meta_modified:String=conn.query_row("SELECT modified FROM sources WHERE id=?1 AND generation=?2",params![metadata.id,metadata.generation],|row|row.get(0)).map_err(sql)?;
    conn.execute_batch("CREATE TEMP TABLE kiro_assigned(id TEXT PRIMARY KEY); BEGIN IMMEDIATE;").map_err(sql)?;
    let result=(|| {
        let mut parser=Projector::new(input(&conn,source,cancel)?,0);let mut checkpoints=0;
        while let Some(mut record)=parser.next(&mut |piece| {
            checkpoint(&conn,source,piece)?;checkpoints+=1;
            if checkpoints>=128 {conn.execute_batch("COMMIT").map_err(sql)?;super::cache::check_wal(store)?;super::cache::trim(store,&conn)?;
                conn.execute_batch("BEGIN IMMEDIATE").map_err(sql)?;checkpoints=0;}Ok(())
        })? {
            if record.error.is_some() {
                if blank(source,&record,cancel)? {continue;}
                return Err(format!("unsupported_record: {}",record.error.unwrap()));
            }
            // Native Kiro snapshots intentionally omit a final newline. A complete JSON value is
            // accepted only with an unchanged paired generation and verified turn metadata below.
            record.complete=true;event(&conn,source,&mut record)?;
            conn.execute("UPDATE sources SET indexed_bytes=max(indexed_bytes,?3) WHERE id=?1 AND generation=?2",params![source.id,source.generation,record.end]).map_err(sql)?;
            conn.execute_batch("COMMIT").map_err(sql)?;super::cache::check_wal(store)?;conn.execute_batch("BEGIN IMMEDIATE").map_err(sql)?;
        }
        boundary(&conn,source,&log_modified)?;let mut parser=Projector::document(input(&conn,metadata,cancel)?,0);
        let header=parser.split_document_array("/session_state/conversation_metadata/user_turn_metadatas",&mut |piece| {
            checkpoint(&conn,metadata,piece)?;checkpoints+=1;
            if checkpoints>=128 {conn.execute_batch("COMMIT").map_err(sql)?;super::cache::check_wal(store)?;super::cache::trim(store,&conn)?;
                conn.execute_batch("BEGIN IMMEDIATE").map_err(sql)?;checkpoints=0;}Ok(())
        },&mut |record| {turn(&conn,source,metadata,&record,cancel)?;conn.execute_batch("COMMIT; BEGIN IMMEDIATE").map_err(sql)?;Ok(())})?;
        if header.end!=metadata.size || header.value["session_id"]!=source.segment.native_id
            || header.value.pointer("/session_state/rts_model_state/conversation_id").and_then(Value::as_str)!=Some(source.segment.native_id.as_str()) {
            return Err("source_changed: Kiro paired session and conversation identities conflict".into());
        }
        if header.value.pointer("/session_state/version").and_then(Value::as_str)!=Some("v1") {return Err("unsupported_record: Kiro metadata version is unsupported".into());}
        let cwd=kiro_store::parse::directory(header.value["cwd"].as_str().ok_or("unsupported_record: Kiro project directory is invalid")?)?;
        let created=kiro_store::parse::millis(&header.value["created_at"])?;let updated=kiro_store::parse::millis(&header.value["updated_at"])?;
        if updated<created {return Err("unsupported_record: Kiro paired time order is invalid".into());}
        let model=&header.value["session_state"]["rts_model_state"]["model_info"];let percent=&header.value["session_state"]["rts_model_state"]["context_usage_percentage"];
        let context=model_context(model,percent)?;boundary(&conn,source,&log_modified)?;boundary(&conn,metadata,&meta_modified)?;
        conn.execute_batch("COMMIT").map_err(sql)?;
        if let Some(database)=&descriptor.database {
            if super::kiro_database::has_session(database,cancel)? {
                super::kiro_database::scan(store,database,cancel)?;
                let raw:String=conn.query_row("SELECT projection FROM records WHERE source_id=?1 AND generation=?2 AND event_type='kiro_metadata'",params![database.id,database.generation],|row|row.get(0)).map_err(sql)?;
                let native:Value=serde_json::from_str(&raw).map_err(|_|"index_incomplete: Kiro database metadata is invalid")?;
                let native_cwd=kiro_store::parse::directory(native["cwd"].as_str().ok_or("index_incomplete: Kiro database project identity is invalid")?)?;
                if cwd.canonicalize().unwrap_or(cwd.clone())!=native_cwd.canonicalize().unwrap_or(native_cwd)
                    || context!=model_context(&native["model"],&native["percent"])? || corpus(&conn,source,cancel)?!=corpus(&conn,database,cancel)? {
                    return Err("source_changed: Kiro history has conflicting sources; no source was selected".into());
                }
            }
        }
        boundary(&conn,source,&log_modified)?;boundary(&conn,metadata,&meta_modified)?;
        conn.execute_batch("BEGIN IMMEDIATE").map_err(sql)?;
        conn.execute("INSERT OR REPLACE INTO records(source_id,generation,start,end,event_type,projection,active) VALUES(?1,?2,0,?3,'kiro_metadata',?4,1)",
            params![metadata.id,metadata.generation,metadata.size,json!({"cwd":cwd,"updatedAt":updated,"model":model,"percent":percent}).to_string()]).map_err(sql)?;
        conn.execute("UPDATE records SET active=1 WHERE source_id=?1 AND generation=?2",params![source.id,source.generation]).map_err(sql)?;
        conn.execute("UPDATE sources SET indexed_bytes=size,status='complete',error=NULL WHERE id=?1 AND generation=?2",params![source.id,source.generation]).map_err(sql)?;
        conn.execute_batch("COMMIT; PRAGMA wal_checkpoint(PASSIVE)").map_err(sql)?;Ok(())
    })();
    if result.is_err() {let _=conn.execute_batch("ROLLBACK");}result
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::api::Branch;
    fn write(f:&kiro_store::tests::Fixture,meta:&Value,log:&[Value]) {
        std::fs::write(f.roots.sessions.join("selected.json"),meta.to_string()).unwrap();
        std::fs::write(f.roots.sessions.join("selected.jsonl"),log.iter().map(Value::to_string).collect::<Vec<_>>().join("\n")).unwrap();
    }
    fn branch(f:&kiro_store::tests::Fixture) -> Branch {
        let store=Store::at(&f.dir).unwrap();let source=register_from(&store,f.roots.sessions.join("selected.jsonl"),"selected",f.roots.database.clone()).unwrap();
        let conn=store.connect().unwrap();conn.execute("INSERT INTO segments VALUES('scope',0,?1,?2,NULL,NULL)",params![source.id,source.generation]).unwrap();
        Branch {store,sources:vec![source],generation:"fixture".into(),kind:SessionKind::Kiro,native_id:"selected".into(),scope:"scope".into(),error:None,parent:None}
    }
    #[test]
    fn paired_long_blocks_keep_concatenation_and_verified_turn_identity_without_a_final_newline() {
        let f=kiro_store::tests::Fixture::new();let long=" 雪😀 paired original\n".repeat(6000);
        let (mut meta,mut log)=kiro_store::tests::pair_values("selected",&f.dir,"First");
        log[0]["data"]["content"]=json!([{"kind":"text","data":"First "},{"kind":"text","data":long}]);
        log[1]["data"]["content"]=json!([{"kind":"text","data":long},{"kind":"text","data":" Last"}]);
        meta["session_state"]["conversation_metadata"]["user_turn_metadatas"][0]["result"]["Ok"]["content"]=log[1]["data"]["content"].clone();write(&f,&meta,&log);
        let before=["selected.json","selected.jsonl"].map(|name|std::fs::read(f.roots.sessions.join(name)).unwrap());let branch=branch(&f);
        assert_eq!(branch.page(None,None,None).unwrap()["rows"],json!([]));scan(&branch.store,&branch.sources[0],&AtomicBool::new(false)).unwrap();
        let page=branch.page(None,None,None).unwrap();assert_eq!(page["coverage"],"complete");assert_eq!(page["knownRows"],2);
        let prose=super::super::prose::Prose::new(branch.clone(),&AtomicBool::new(false)).unwrap();let messages=prose.collect_bounded(0,2,512*1024,&AtomicBool::new(false)).unwrap();
        let context=super::super::context::kiro_branch(&branch).unwrap();assert_eq!(context.model.as_deref(),Some("native-model"));assert_eq!(context.percent,Some(12.5));
        assert_eq!(messages[0].text,format!("First {long}"));assert_eq!(messages[1].text,format!("{long} Last"));
        let id=page["rows"][0]["id"].as_str().unwrap();let manifest=branch.manifest(id,0).unwrap();assert_eq!(manifest["fields"].as_array().unwrap().len(),2);
        assert!(manifest["fields"].as_array().unwrap().iter().all(|field|field["append"]==true));
        for (name,expected) in ["selected.json","selected.jsonl"].into_iter().zip(before) {assert_eq!(std::fs::read(f.roots.sessions.join(name)).unwrap(),expected);}
    }
    #[test]
    fn mismatched_turn_content_missing_ids_and_duplicate_assignments_keep_staged_rows_invisible() {
        for mutation in ["content","missing","assigned","identity","version"] {
            let f=kiro_store::tests::Fixture::new();let (mut meta,log)=kiro_store::tests::pair_values("selected",&f.dir,"Prompt");
            match mutation {
                "content"=>meta["session_state"]["conversation_metadata"]["user_turn_metadatas"][0]["result"]["Ok"]["content"][0]["data"]=json!("Different"),
                "missing"=>meta["session_state"]["conversation_metadata"]["user_turn_metadatas"][0]["message_ids"]=json!(["response-other-namespace"]),
                "assigned"=>{let turn=meta["session_state"]["conversation_metadata"]["user_turn_metadatas"][0].clone();meta["session_state"]["conversation_metadata"]["user_turn_metadatas"].as_array_mut().unwrap().push(turn);},
                "identity"=>meta["session_id"]=json!("different"),"version"=>meta["session_state"]["version"]=json!("future"),_=>unreachable!(),
            }write(&f,&meta,&log);let branch=branch(&f);assert!(scan(&branch.store,&branch.sources[0],&AtomicBool::new(false)).is_err(),"{mutation}");
            assert_eq!(branch.page(None,None,None).unwrap()["rows"],json!([]),"{mutation}");
        }
    }
    #[test]
    fn paired_and_database_sources_require_identical_full_messages_and_context() {
        for matches in [true,false] {
            let f=kiro_store::tests::Fixture::new();let native=f.db();f.insert(&native,"selected",&f.dir,if matches {"Prompt"} else {"Different"});
            f.pair("selected",&f.dir,"Prompt");let branch=branch(&f);let result=scan(&branch.store,&branch.sources[0],&AtomicBool::new(false));
            if matches {result.unwrap();assert_eq!(branch.page(None,None,None).unwrap()["knownRows"],2);}
            else {assert!(result.unwrap_err().contains("conflicting sources"));assert_eq!(branch.page(None,None,None).unwrap()["rows"],json!([]));}
        }
    }
    #[test]
    fn consecutive_native_messages_keep_their_existing_message_numbers() {
        let f=kiro_store::tests::Fixture::new();let (mut meta,mut log)=kiro_store::tests::pair_values("selected",&f.dir,"Prompt");
        let mut next=log[1].clone();next["data"]["message_id"]=json!("assistant-two");log.push(next);
        meta["session_state"]["conversation_metadata"]["user_turn_metadatas"][0]["message_ids"]=json!(["user-one","assistant-one","assistant-two"]);
        write(&f,&meta,&log);let branch=branch(&f);scan(&branch.store,&branch.sources[0],&AtomicBool::new(false)).unwrap();
        let prose=super::super::prose::Prose::new(branch,&AtomicBool::new(false)).unwrap();assert_eq!(prose.total,3);
        let messages=prose.collect_bounded(0,3,512*1024,&AtomicBool::new(false)).unwrap();
        assert_eq!(messages.iter().map(|message|message.text.as_str()).collect::<Vec<_>>(),["Prompt","Reply","Reply"]);
    }
    #[test]
    fn many_short_blocks_are_read_as_full_content_after_the_record_preview_budget_is_exhausted() {
        let f=kiro_store::tests::Fixture::new();let (meta,mut log)=kiro_store::tests::pair_values("selected",&f.dir,"Prompt");
        let block="x".repeat(2048);log[0]["data"]["content"]=json!((0..100).map(|_|json!({"kind":"text","data":block})).collect::<Vec<_>>());
        write(&f,&meta,&log);let branch=branch(&f);scan(&branch.store,&branch.sources[0],&AtomicBool::new(false)).unwrap();
        let prose=super::super::prose::Prose::new(branch,&AtomicBool::new(false)).unwrap();
        assert_eq!(prose.collect_bounded(0,1,512*1024,&AtomicBool::new(false)).unwrap()[0].text,block.repeat(100));
    }
}

//! Native SQLite bodies stay in their owning database. The index stores bounded projections,
//! entry identities and verified byte ranges in a logical concatenation of selected columns.

use std::io::{Read,Seek,SeekFrom};
use std::path::{Path,PathBuf};
use std::sync::atomic::{AtomicBool,Ordering};
use rusqlite::{params,Connection,DatabaseName,OptionalExtension};
use serde_json::{json,Value};
use sha2::{Digest,Sha256};
use crate::models::SessionKind;
use super::{adapter,limits::*,projection::{Checkpoint,Projector,Record},source::{self,Segment},sqlite_source::Entry,store::{sql,RegisteredSource,Store}};

#[derive(Clone,Debug)]
pub(super) struct Snapshot {pub count:u64,pub manifest:Vec<u8>,pub cutoff:Option<(i64,String)>}
struct Metadata {kind:i64,rowid:i64,id:String,message:String,bytes:u64,signature:String}

fn updated(conn:&Connection,table:&str) -> Result<bool,String> {
    let mut statement=conn.prepare(&format!("PRAGMA table_info({table})")).map_err(sql)?;
    let rows=statement.query_map([],|row|row.get::<_,String>(1)).map_err(sql)?;
    for row in rows {if row.map_err(sql)?=="time_updated" {return Ok(true);}}Ok(false)
}
fn metadata_sql(conn:&Connection) -> Result<String,String> {
    let message_updated=if updated(conn,"message")? {"time_updated"} else {"NULL"};
    let part_updated=if updated(conn,"part")? {"p.time_updated"} else {"NULL"};
    Ok(format!("WITH visible AS (SELECT rowid AS native_rowid,CASE WHEN octet_length(id)<=200 THEN id END AS id,
            session_id,time_created,octet_length(data) AS data_bytes,{message_updated} AS updated FROM message
            WHERE session_id=?1 AND (?2 IS NULL OR time_created<?2 OR (time_created=?2 AND id<?3)))
        SELECT 0 AS kind,m.native_rowid,CASE WHEN octet_length(m.id)<=200 THEN m.id END AS id,
            CASE WHEN octet_length(m.id)<=200 THEN m.id END AS message_id,m.data_bytes,
            m.time_created AS message_time,m.id AS message_order,0 AS phase,0 AS part_time,m.updated AS updated
            FROM visible m
        UNION ALL
        SELECT 1,p.rowid,CASE WHEN octet_length(p.id)<=200 THEN p.id END,
            CASE WHEN octet_length(p.message_id)<=200 THEN p.message_id END,octet_length(p.data),m.time_created,m.id,1,p.time_created,{part_updated}
            FROM part p JOIN visible m ON m.id=p.message_id AND p.session_id=m.session_id
        ORDER BY message_time,message_order,phase,part_time,id LIMIT ?4"))
}
fn metadata(row:&rusqlite::Row<'_>) -> Result<Metadata,String> {
    let kind:i64=row.get(0).map_err(sql)?;let rowid:i64=row.get(1).map_err(sql)?;
    let id:Option<String>=row.get(2).map_err(sql)?;let message:Option<String>=row.get(3).map_err(sql)?;
    let id=id.filter(|id|!id.is_empty()).ok_or("unsupported_record: native database identity exceeds its metadata budget")?;
    let message=message.filter(|id|!id.is_empty()).ok_or("unsupported_record: native database message identity is unavailable")?;
    let bytes:u64=row.get(4).map_err(sql)?;
    let signature=json!([kind,rowid,id,message,bytes,row.get::<_,i64>(5).map_err(sql)?,row.get::<_,i64>(8).map_err(sql)?,row.get::<_,Option<i64>>(9).map_err(sql)?]).to_string();
    Ok(Metadata {kind,rowid,id,message,bytes,signature})
}
fn cutoff(conn:&Connection,session:&str) -> Result<Option<(i64,String)>,String> {
    let revert:Option<(Option<u64>,Option<String>)>=conn.query_row("SELECT octet_length(revert),CASE WHEN octet_length(revert)<=65536 THEN revert END FROM session WHERE id=?1",
        [session],|row|Ok((row.get(0)?,row.get(1)?))).optional().map_err(sql)?;
    let (bytes,raw)=revert.ok_or("source_missing: OpenCode session is unavailable")?;
    if bytes.is_some_and(|bytes|bytes>65536) {return Err("unsupported_record: OpenCode revert metadata exceeds its byte budget".into());}
    let value=raw.map(|raw|serde_json::from_str::<Value>(&raw).map_err(|_|"unsupported_record: OpenCode revert metadata is invalid".to_string())).transpose()?;
    let Some(id)=value.as_ref().and_then(crate::agent::opencode_store::reverted_from) else {return Ok(None);};
    if id.len()>200 {return Err("unsupported_record: OpenCode revert identity exceeds its metadata budget".into());}
    conn.query_row("SELECT time_created,id FROM message WHERE session_id=?1 AND id=?2",params![session,id],|row|Ok((row.get(0)?,row.get(1)?)))
        .optional().map_err(sql)?.map(Some).ok_or_else(||"index_incomplete: OpenCode revert boundary is unavailable".into())
}
fn inspect(conn:&Connection,session:&str,cut:&Option<(i64,String)>,limit:u64,prefix_count:u64) -> Result<(u64,u64,Vec<u8>,Vec<u8>),String> {
    let mut statement=conn.prepare(&metadata_sql(conn)?).map_err(sql)?;
    let mut rows=statement.query(params![session,cut.as_ref().map(|item|item.0),cut.as_ref().map(|item|&item.1),limit.min(i64::MAX as u64) as i64]).map_err(sql)?;
    let mut hash=Sha256::new();let mut prefix=Sha256::new();let mut count=0;let mut bytes=0u64;
    while let Some(row)=rows.next().map_err(sql)? {
        let entry=metadata(row)?;hash.update(entry.signature.as_bytes());hash.update(b"\n");
        if count<prefix_count {prefix.update(entry.signature.as_bytes());prefix.update(b"\n");}
        bytes=bytes.checked_add(entry.bytes+1).ok_or("unsupported_record: native database byte count overflow")?;count+=1;
    }
    Ok((count,bytes,hash.finalize().to_vec(),prefix.finalize().to_vec()))
}
pub(super) fn stamp(path:&Path) -> Result<String,String> {
    let mut facts=Vec::new();
    for candidate in [path.to_owned(),PathBuf::from(format!("{}-wal",path.to_string_lossy()))] {
        match std::fs::metadata(&candidate) {
            Ok(metadata)=>facts.push(json!([source::identity(&metadata),metadata.len(),format!("{:?}",metadata.modified().ok())])),
            Err(error) if error.kind()==std::io::ErrorKind::NotFound=>facts.push(Value::Null),
            Err(_)=>return Err("source_missing: native database metadata is unavailable".into()),
        }
    }Ok(json!(facts).to_string())
}

pub(super) fn register(store:&Store,path:PathBuf,native:&str) -> Result<RegisteredSource,String> {register_attempt(store,path,native,0)}
fn register_attempt(store:&Store,path:PathBuf,native:&str,attempt:usize) -> Result<RegisteredSource,String> {
    if native.is_empty() || native.len()>200 {return Err("detail_unavailable: native database session identity is invalid".into());}
    let identity=source::identity(&std::fs::metadata(&path).map_err(|_|"source_missing: OpenCode database is unavailable")?);
    let modified=stamp(&path)?;let native_conn=super::sqlite_source::open(&path)?;let cut=cutoff(&native_conn,native)?;
    let conn=store.connect()?;
    let key=format!("sqlite:opencode:{}:{native}",path.to_string_lossy());
    let previous:Option<(String,String,String,String,Vec<u8>,u64,u64,String,u32,String)>=conn.query_row("SELECT s.id,s.generation,s.identity,s.modified,s.head_hash,n.entry_count,s.indexed_bytes,s.status,s.parser_version,s.branch_revision
        FROM sources s JOIN native_db_sources n ON n.source_id=s.id WHERE s.path=?1",[&key],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?,row.get(6)?,row.get(7)?,row.get(8)?,row.get(9)?))).optional().map_err(sql)?;
    let old_count=previous.as_ref().map(|previous|previous.5).unwrap_or(0);
    let (count,bytes,manifest,prefix)=inspect(&native_conn,native,&cut,u64::MAX,old_count)?;
    drop(native_conn);
    let observed=previous.as_ref().map(|item|(item.0.clone(),item.1.clone(),item.3.clone(),item.5));
    let branch=super::store::digest(json!(cut).to_string().as_bytes());
    let (id,generation,mut indexed,mut status)=if let Some((id,generation,old_identity,old_modified,old_hash,old_count,indexed,status,version,old_branch))=previous {
        if old_identity==identity && version==PARSER_VERSION && old_branch==branch && count>=old_count && old_hash==prefix
            && (count>old_count || modified==old_modified) {
            (id,generation,indexed,if count>old_count {"partial".into()} else {status})
        } else {(id,uuid::Uuid::new_v4().to_string(),0,"partial".into())}
    } else {(uuid::Uuid::new_v4().to_string(),uuid::Uuid::new_v4().to_string(),0,"partial".into())};
    let _gate=store.cache_gate.lock().unwrap();
    let current:Option<(String,String,String,u64)>=conn.query_row("SELECT s.id,s.generation,s.modified,n.entry_count FROM sources s
        JOIN native_db_sources n ON n.source_id=s.id WHERE s.path=?1",[&key],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?))).optional().map_err(sql)?;
    if current!=observed {
        drop(_gate);drop(conn);
        if attempt>=3 {return Err("index_busy: native history is being updated by another reader".into());}
        return register_attempt(store,path,native,attempt+1);
    }
    conn.execute_batch("BEGIN IMMEDIATE").map_err(sql)?;
    let result=(|| {
        // A scanner can commit while native metadata is inspected. Keep its durable water mark when
        // the observed generation is still current, rather than replaying already merged user parts.
        if let Some((water,state))=conn.query_row("SELECT indexed_bytes,status FROM sources WHERE id=?1 AND generation=?2",
            params![id,generation],|row|Ok((row.get::<_,u64>(0)?,row.get::<_,String>(1)?))).optional().map_err(sql)? {
            indexed=indexed.max(water);if indexed>=bytes {status=state;}
        }
        conn.execute("INSERT INTO sources(id,provider,native_id,path,identity,generation,size,modified,head_hash,indexed_bytes,status,parser_version,branch_revision)
            VALUES(?1,'opencode',?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12) ON CONFLICT(path) DO UPDATE SET identity=excluded.identity,
            generation=excluded.generation,size=excluded.size,modified=excluded.modified,head_hash=excluded.head_hash,indexed_bytes=excluded.indexed_bytes,
            status=excluded.status,parser_version=excluded.parser_version,branch_revision=excluded.branch_revision,error=NULL",
            params![id,native,key,identity,generation,bytes,modified,manifest,indexed,status,PARSER_VERSION,branch]).map_err(sql)?;
        conn.execute("INSERT INTO native_db_sources VALUES(?1,?2,?3,?4) ON CONFLICT(source_id) DO UPDATE SET path=excluded.path,native_id=excluded.native_id,entry_count=excluded.entry_count",
            params![id,path.to_string_lossy(),native,count]).map_err(sql)?;
        conn.execute("INSERT INTO generation_access VALUES(?1,?2,?3) ON CONFLICT(source_id,generation) DO UPDATE SET touched=excluded.touched",
            params![id,generation,super::cache::now()]).map_err(sql)?;Ok(())
    })();
    if result.is_err() {let _=conn.execute_batch("ROLLBACK");return result.map(|_|unreachable!());}conn.execute_batch("COMMIT").map_err(sql)?;
    for (key,cancel) in store.jobs.lock().unwrap().iter() {
        if key.starts_with(&format!("{id}:")) && key!=&format!("{id}:{generation}") {cancel.store(true,Ordering::Relaxed);}
    }
    let _pin=super::cache::Pin::new(store,format!("{id}:{generation}"));super::cache::trim_locked(store,&conn)?;
    Ok(RegisteredSource {id,generation,segment:Segment {path,native_id:native.into(),end_byte:None,end_ordinal:None},size:bytes,indexed,status,
        database:Some(Snapshot {count,manifest,cutoff:cut}),kiro:None,kiro_pair:None,_pin})
}

struct BlobReader<'a> {blob:rusqlite::blob::Blob<'a>,conn:&'a Connection,source:&'a RegisteredSource,cancel:&'a AtomicBool,offset:u64}
impl Read for BlobReader<'_> {
    fn read(&mut self,bytes:&mut [u8]) -> std::io::Result<usize> {
        if self.cancel.load(Ordering::Relaxed) {return Err(std::io::Error::other("read_canceled"));}
        let count=self.blob.read(bytes)?;if count==0 {return Ok(0);}
        let hash=Sha256::digest(&bytes[..count]).to_vec();
        let previous:Option<(u64,Vec<u8>)>=self.conn.query_row("SELECT end,hash FROM source_blocks WHERE source_id=?1 AND generation=?2 AND start=?3",
            params![self.source.id,self.source.generation,self.offset],|row|Ok((row.get(0)?,row.get(1)?))).optional().map_err(std::io::Error::other)?;
        if previous.is_some_and(|(end,previous)|end==self.offset+count as u64 && previous!=hash) {return Err(std::io::Error::other("source_changed: native database entry changed"));}
        self.conn.execute("INSERT OR REPLACE INTO source_blocks VALUES(?1,?2,?3,?4,?5)",params![self.source.id,self.source.generation,self.offset,self.offset+count as u64,hash])
            .map_err(std::io::Error::other)?;self.offset+=count as u64;Ok(count)
    }
}

fn checkpoint(conn:&Connection,source:&RegisteredSource,prefix:&str,checkpoint:Checkpoint) -> Result<(),String> {
    conn.execute("INSERT OR REPLACE INTO checkpoints VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",params![source.id,source.generation,checkpoint.record_start,
        format!("{prefix}{}",checkpoint.path),checkpoint.raw_start,checkpoint.raw_end,checkpoint.decoded_start,checkpoint.decoded_end,checkpoint.hash,checkpoint.sequence]).map_err(sql)?;Ok(())
}

fn info(value:&Value,id:&str) -> Value {
    let mut result=serde_json::Map::new();for key in ["role","modelID","summary","agent","mode","time","error","parentID"] {
        if let Some(value)=value.get(key) {result.insert(key.into(),value.clone());}
    }result.insert("id".into(),json!(id));Value::Object(result)
}

pub(super) fn scan(store:&Store,source:&RegisteredSource,cancel:&AtomicBool) -> Result<(),String> {
    let snapshot=source.database.as_ref().ok_or("unsupported_record: native database snapshot is missing")?;
    let native=super::sqlite_source::open(&source.segment.path)?;
    let (_,bytes,manifest,_)=inspect(&native,&source.segment.native_id,&snapshot.cutoff,snapshot.count,0)?;
    if bytes!=source.size || manifest!=snapshot.manifest {return Err("source_changed: native database entries changed before scanning".into());}
    let conn=store.connect()?;let (expected,indexed):(String,u64)=conn.query_row("SELECT identity,indexed_bytes FROM sources WHERE id=?1 AND generation=?2",
        params![source.id,source.generation],|row|Ok((row.get(0)?,row.get(1)?))).map_err(sql)?;
    let mut statement=native.prepare(&metadata_sql(&native)?).map_err(sql)?;
    let mut entries=statement.query(params![source.segment.native_id,snapshot.cutoff.as_ref().map(|item|item.0),snapshot.cutoff.as_ref().map(|item|&item.1),snapshot.count.min(i64::MAX as u64) as i64]).map_err(sql)?;
    let mut start=0u64;let mut current_id=String::new();let mut current_info=Value::Null;
    while let Some(row)=entries.next().map_err(sql)? {
        if cancel.load(Ordering::Relaxed) {return Err("read_canceled".into());}
        let entry=metadata(row)?;let end=start+entry.bytes+1;
        if end<=indexed {start=end;continue;}
        let generation:String=conn.query_row("SELECT generation FROM sources WHERE id=?1",[&source.id],|row|row.get(0)).map_err(sql)?;
        if generation!=source.generation {return Err("source_changed: native database generation changed".into());}
        if source::identity(&std::fs::metadata(&source.segment.path).map_err(|_|"source_changed: native database was removed")?)!=expected {
            return Err("source_changed: native database was replaced".into());
        }
        let kind=if entry.kind==0 {Entry::OpenCodeMessage} else {Entry::OpenCodePart};
        let blob=native.blob_open(DatabaseName::Main,kind.table(),kind.column(),entry.rowid,true).map_err(|_|"unsupported_record: native database column cannot be read incrementally")?;
        let prefix=if entry.kind==0 {"/info"} else {"/part"};
        conn.execute_batch("BEGIN IMMEDIATE").map_err(sql)?;
        let result=(|| {
            let reader=BlobReader {blob,conn:&conn,source,cancel,offset:start};let mut parser=Projector::document(reader,start);let mut checkpoints=0;
            let mut record=parser.next(&mut |piece| {
                checkpoint(&conn,source,prefix,piece)?;checkpoints+=1;
                if checkpoints>=128 {conn.execute_batch("COMMIT").map_err(sql)?;super::cache::trim(store,&conn)?;super::cache::check_wal(store)?;
                    conn.execute_batch("BEGIN IMMEDIATE").map_err(sql)?;checkpoints=0;std::thread::yield_now();}Ok(())
            })?.ok_or("unsupported_record: native database column is empty")?;
            if record.end!=end-1 {return Err("unsupported_record: native database column contains trailing records".into());}
            if source::identity(&std::fs::metadata(&source.segment.path).map_err(|_|"source_changed: native database was removed during scanning")?)!=expected {
                return Err("source_changed: native database was replaced during scanning".into());
            }
            record.complete=true;record.end=end;for field in &mut record.fields {field.path=format!("{prefix}{}",field.path);}
            if entry.kind==0 {
                current_info=info(&record.value,&entry.id);current_id=entry.id.clone();record.value=json!({"type":"opencode_message","info":current_info});
            } else {
                if current_id!=entry.message {
                    let raw:String=conn.query_row("SELECT r.projection FROM records r JOIN native_db_entries e ON e.source_id=r.source_id AND e.generation=r.generation AND e.record_start=r.start
                        WHERE e.source_id=?1 AND e.generation=?2 AND e.entry_kind=0 AND e.entry_id=?3",params![source.id,source.generation,entry.message],|row|row.get(0)).map_err(sql)?;
                    current_info=serde_json::from_str::<Value>(&raw).map_err(|_|"index_incomplete: native message metadata is invalid")?["info"].clone();current_id=entry.message.clone();
                }
                let mut part=record.value;part["id"]=json!(entry.id);part["messageID"]=json!(entry.message);part["sessionID"]=json!(source.segment.native_id);
                record.value=json!({"type":"opencode_part","info":current_info,"part":part});
            }
            conn.execute("INSERT OR REPLACE INTO native_db_entries VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",params![source.id,source.generation,start,end,entry.kind,entry.rowid,entry.id,entry.message,entry.bytes]).map_err(sql)?;
            let refs=adapter::store_fields(&conn,source,&record)?;store_record(&conn,source,&record,&refs)?;
            conn.execute("UPDATE sources SET indexed_bytes=max(indexed_bytes,?3) WHERE id=?1 AND generation=?2",params![source.id,source.generation,end]).map_err(sql)?;
            conn.execute_batch("COMMIT").map_err(sql)?;Ok(())
        })();
        if result.is_err() {let _=conn.execute_batch("ROLLBACK");return result;}
        super::cache::check_wal(store)?;start=end;
    }
    let gaps:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM records WHERE source_id=?1 AND generation=?2 AND error IS NOT NULL)",
        params![source.id,source.generation],|row|row.get(0)).map_err(sql)?;
    conn.execute("UPDATE sources SET status=CASE WHEN ?3 THEN 'error' WHEN indexed_bytes>=size THEN 'complete' ELSE 'partial' END
        WHERE id=?1 AND generation=?2",params![source.id,source.generation,gaps]).map_err(sql)?;
    conn.execute_batch("PRAGMA wal_checkpoint(PASSIVE)").map_err(sql)?;Ok(())
}

pub(super) fn body_paths(record:&Record,role:&str) -> Vec<String> {
    let roots:Vec<&str>=match role {"toolInput"=>vec!["/part/state/input"],"toolOutput"=>vec!["/part/state/output","/part/state/error"],_=>vec!["/part/text"]};
    record.fields.iter().filter(|field|roots.iter().any(|root|field.path==*root || field.path.starts_with(&format!("{root}/"))))
        .map(|field|field.path.clone()).collect()
}
fn store_record(conn:&Connection,source:&RegisteredSource,record:&Record,refs:&std::collections::HashMap<String,String>) -> Result<(),String> {
    conn.execute("INSERT OR REPLACE INTO records(source_id,generation,start,end,event_type,projection,error) VALUES(?1,?2,?3,?4,?5,?6,?7)",
        params![source.id,source.generation,record.start,record.end,record.value["type"].as_str(),record.value.to_string(),record.error]).map_err(sql)?;
    if let Some(error)=&record.error {return adapter::write_row(conn,source,record,0,None,&json!({"id":format!("gap:{}:{}:{}",source.id,source.generation,record.start),"kind":"notice","message":error}));}
    let message=crate::agent::opencode_store::OpencodeMessage {info:record.value["info"].clone(),parts:if record.value["type"]=="opencode_part" {vec![record.value["part"].clone()]} else {vec![]}};
    for (local,row) in crate::agent::chat::opencode_timeline::rows(&[message],&|_|None).into_iter().enumerate() {
        let mut row=serde_json::to_value(row).map_err(|e|e.to_string())?;let native=row["id"].as_str().unwrap_or_default().to_owned();
        let id=format!("native:{}",super::store::digest(format!("opencode:{}:{native}",source.segment.native_id).as_bytes()));row["id"]=json!(id);
        let tool=row["kind"]=="tool";
        let mut paths=body_paths(record,if tool {"toolInput"} else {"body"});let mut label=if tool {"input"} else {"body"};
        let file=row["kind"]=="user" && record.value.pointer("/part/type").and_then(Value::as_str)==Some("file");
        if file {
            let image=record.value.pointer("/part/mime").and_then(Value::as_str).is_some_and(|mime|mime.starts_with("image/"));
            let path=if image || record.value.pointer("/part/filename").is_none() {"/part/mime"} else {"/part/filename"};
            paths=vec![path.into()];label=if image {"body_image"} else {"body_file"};
        }
        let mut size=record.fields.iter().filter(|field|paths.contains(&field.path) && field.encoding=="string").map(|field|field.decoded_bytes).sum::<u64>();
        if file {size=if label=="body_image" {7} else {size+8};}
        if tool {row["input"]=adapter::short_input(&row["input"]);row["detailAvailable"]=json!(true);if let Some(output)=row["output"].as_str() {row["output"]=json!(adapter::preview(output,512));}}
        else if let Some(text)=row["text"].as_str() {
            let text=adapter::preview(text,PREVIEW_BYTES);row["text"]=json!(text);
            if row["kind"]=="user" {
                let old:Option<String>=conn.query_row("SELECT summary FROM rows WHERE source_id=?1 AND generation=?2 AND id=?3",params![source.id,source.generation,id],|row|row.get(0)).optional().map_err(sql)?;
                if let Some(old)=old {
                    let old:Value=serde_json::from_str(&old).map_err(|e|e.to_string())?;let mut text=old["text"].as_str().unwrap_or_default().to_owned();
                    if !text.is_empty() {text.push('\n');}text.push_str(row["text"].as_str().unwrap_or_default());row["text"]=json!(adapter::preview(&text,PREVIEW_BYTES));
                    size=size.saturating_add(old["bodySizeBytes"].as_u64().unwrap_or(0)).saturating_add(1);
                }
            }
            row["bodySizeBytes"]=json!(size);if size>PREVIEW_BYTES as u64 {row["bodyAvailable"]=json!(true);}
        }
        super::children::annotate(SessionKind::Opencode,record,&mut row)?;
        adapter::write_row(conn,source,record,local,record.value.pointer("/part/callID").and_then(Value::as_str),&row)?;
        adapter::attach(conn,source,&id,label,&paths,refs)?;
        if row["kind"]=="user" {
            for path in &paths {if let Some(reference)=refs.get(path) {
                conn.execute("INSERT OR IGNORE INTO append_fields VALUES(?1)",[reference]).map_err(sql)?;
            }}
        }
        if tool {
            adapter::attach(conn,source,&id,"output",&body_paths(record,"toolOutput"),refs)?;
            adapter::attach(conn,source,&id,"original",&["/part".into()],refs)?;
        }
    }restore_shell_row(conn,source,record,refs)?;Ok(())
}

fn row_id(source:&RegisteredSource,message:&str) -> String {
    format!("native:{}",super::store::digest(format!("opencode:{}:u:{message}",source.segment.native_id).as_bytes()))
}
pub(super) fn user_parts(conn:&Connection,source:&RegisteredSource,message:&str,mut sink:impl FnMut(&Value,Option<&str>)->Result<(),String>) -> Result<bool,String> {
    let mut statement=conn.prepare("SELECT r.projection,f.ref FROM native_db_entries e JOIN records r
        ON r.source_id=e.source_id AND r.generation=e.generation AND r.start=e.record_start
        LEFT JOIN fields f ON f.source_id=r.source_id AND f.generation=r.generation AND f.record_start=r.start AND f.path=CASE WHEN json_extract(r.projection,'$.part.type')='file'
            THEN CASE WHEN json_extract(r.projection,'$.part.mime') LIKE 'image/%' OR json_type(r.projection,'$.part.filename') IS NULL THEN '/part/mime' ELSE '/part/filename' END
            ELSE '/part/text' END
        WHERE e.source_id=?1 AND e.generation=?2 AND e.message_id=?3 AND e.entry_kind=1 AND r.error IS NULL
        AND r.end<=?4 ORDER BY r.start").map_err(sql)?;
    let values=statement.query_map(params![source.id,source.generation,message,source.segment.end_byte.unwrap_or(source.size)],
        |row|Ok((row.get::<_,String>(0)?,row.get::<_,Option<String>>(1)?))).map_err(sql)?;
    let mut shell=false;
    for value in values {
        let (raw,reference)=value.map_err(sql)?;let value:Value=serde_json::from_str(&raw).map_err(|_|"index_incomplete: native part metadata is invalid")?;
        let part=&value["part"];
        shell|=part["type"]=="text" && part["synthetic"]==true && part["text"]=="The following tool was executed by the user";
        if part["type"]=="text" && (part["synthetic"]==true || part["ignored"]==true) {continue;}
        let text=crate::agent::chat::opencode_timeline::user_text(std::slice::from_ref(part));
        // An empty prefix of a long field cannot prove that the entire original is whitespace.
        let long=if let Some(reference)=&reference {conn.query_row("SELECT decoded_bytes>?2 FROM fields WHERE ref=?1",
            params![reference,PREVIEW_BYTES],|row|row.get::<_,bool>(0)).map_err(sql)?} else {false};
        if !text.is_empty() || long {sink(part,reference.as_deref())?;}
    }Ok(shell)
}
pub(super) fn shell_command(conn:&Connection,source:&RegisteredSource,message:&str) -> Result<Option<(Value,String)>,String> {
    conn.query_row("SELECT r.projection,f.ref FROM records r JOIN fields f ON f.source_id=r.source_id AND f.generation=r.generation AND f.record_start=r.start
        WHERE r.source_id=?1 AND r.generation=?2 AND r.end<=?3 AND r.error IS NULL AND r.event_type='opencode_part'
        AND json_extract(r.projection,'$.info.role')='assistant' AND json_extract(r.projection,'$.info.parentID')=?4
        AND json_extract(r.projection,'$.part.type')='tool' AND json_extract(r.projection,'$.part.tool')='bash'
        AND f.path='/part/state/input/command' ORDER BY r.start LIMIT 1",
        params![source.id,source.generation,source.segment.end_byte.unwrap_or(source.size),message],|row|Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?)))
        .optional().map_err(sql)?.map(|(raw,reference)|serde_json::from_str(&raw).map(|value|(value,reference)).map_err(|_|"index_incomplete: native shell metadata is invalid".into())).transpose()
}
pub(super) fn user_preview(conn:&Connection,source:&RegisteredSource,message:&str) -> Result<Option<String>,String> {
    let mut text=String::new();let mut any=false;
    let shell=user_parts(conn,source,message,|part,_| {
        if any && text.len()<PREVIEW_BYTES {text.push('\n');}any=true;
        let part=crate::agent::chat::opencode_timeline::user_text(std::slice::from_ref(part));
        text.push_str(&adapter::preview(&part,PREVIEW_BYTES.saturating_sub(text.len())));Ok(())
    })?;
    if any {return Ok(Some(text));}
    if shell {if let Some((part,_))=shell_command(conn,source,message)? {
        return Ok(Some(format!("!{}",adapter::preview(part.pointer("/part/state/input/command").and_then(Value::as_str).unwrap_or_default(),PREVIEW_BYTES-1))));
    }}Ok(None)
}
pub(super) fn walk_user(branch:&super::api::Branch,source:&RegisteredSource,message:&str,cancel:&AtomicBool,sink:&mut impl FnMut(&str)->Result<(),String>) -> Result<(),String> {
    let conn=branch.store.connect()?;let mut any=false;
    // Recreate the native multipart message with one bounded descriptor at a time.
    let shell=user_parts(&conn,source,message,|part,reference| {
        if cancel.load(Ordering::Relaxed) {return Err("read_canceled".into());}
        if any {sink("\n")?;}any=true;
        if part["type"]=="text" {
            let reference=reference.ok_or("index_incomplete: original user text is unavailable")?;
            let mut sequence=0;loop {let (text,_,end,size)=super::api::read_piece(&conn,source,reference,sequence,0)?;sink(&text)?;
                if cancel.load(Ordering::Relaxed) {return Err("read_canceled".into());}if end>=size {break;}sequence+=1;}
        } else {
            let image=part["mime"].as_str().is_some_and(|mime|mime.starts_with("image/"));
            if !image {sink("[file: ")?;}
            if let Some(reference)=reference {let mut sequence=0;loop {
                let (text,_,end,size)=super::api::read_piece(&conn,source,reference,sequence,0)?;
                if !image {sink(&text)?;}
                if cancel.load(Ordering::Relaxed) {return Err("read_canceled".into());}if end>=size {break;}sequence+=1;
            }}
            sink(if image {"[image]"} else {"]"})?;
        }Ok(())
    })?;
    if !any && shell {if let Some((_,reference))=shell_command(&conn,source,message)? {
        sink("!")?;let mut sequence=0;loop {let (text,_,end,size)=super::api::read_piece(&conn,source,&reference,sequence,0)?;sink(&text)?;
            if cancel.load(Ordering::Relaxed) {return Err("read_canceled".into());}if end>=size {break;}sequence+=1;}
    }}Ok(())
}
fn restore_shell_row(conn:&Connection,source:&RegisteredSource,record:&Record,refs:&std::collections::HashMap<String,String>) -> Result<(),String> {
    if record.value.pointer("/part/type").and_then(Value::as_str)!=Some("tool") || record.value.pointer("/part/tool").and_then(Value::as_str)!=Some("bash") {return Ok(());}
    let Some(parent)=record.value.pointer("/info/parentID").and_then(Value::as_str) else {return Ok(());};
    let id=row_id(source,parent);
    let exists:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM rows WHERE source_id=?1 AND generation=?2 AND id=?3)",params![source.id,source.generation,id],|row|row.get(0)).map_err(sql)?;
    if exists {return Ok(());}
    let Some(text)=user_preview(conn,source,parent)? else {return Ok(());};
    let Some(reference)=refs.get("/part/state/input/command") else {return Ok(());};
    let origin:Option<(u64,String)>=conn.query_row("SELECT r.start,r.projection FROM native_db_entries e JOIN records r
        ON r.source_id=e.source_id AND r.generation=e.generation AND r.start=e.record_start
        WHERE e.source_id=?1 AND e.generation=?2 AND e.entry_id=?3 AND e.entry_kind=0",params![source.id,source.generation,parent],|row|Ok((row.get(0)?,row.get(1)?))).optional().map_err(sql)?;
    let Some((start,info))=origin else {return Ok(());};let info:Value=serde_json::from_str(&info).map_err(|_|"index_incomplete: native user metadata is invalid")?;
    let size:u64=conn.query_row("SELECT decoded_bytes FROM fields WHERE ref=?1",[reference],|row|row.get(0)).map_err(sql)?;
    let row=json!({"id":id,"kind":"user","text":text,"images":[],"at":info.pointer("/info/time/created"),"bodySizeBytes":size+1,"bodyAvailable":size+1>PREVIEW_BYTES as u64});
    adapter::write_row(conn,source,record,0,None,&row)?;
    conn.execute("UPDATE rows SET order_key=?4 WHERE source_id=?1 AND generation=?2 AND id=?3",params![source.id,source.generation,id,format!("{start:020}:000000")]).map_err(sql)?;
    adapter::attach(conn,source,&id,"body_shell",&["/part/state/input/command".into()],refs)?;Ok(())
}

pub(super) fn range(conn:&Connection,source:&RegisteredSource,start:u64,end:u64) -> Result<Vec<u8>,String> {
    let expected:Option<String>=conn.query_row("SELECT identity FROM sources WHERE id=?1 AND generation=?2",params![source.id,source.generation],|row|row.get(0)).optional().map_err(sql)?;
    if expected.is_none() || expected.as_deref()!=std::fs::metadata(&source.segment.path).ok().as_ref().map(source::identity).as_deref() {
        return Err("source_changed: native database identity changed".into());
    }
    let native=super::sqlite_source::open(&source.segment.path)?;let mut output=Vec::with_capacity((end-start) as usize);let mut at=start;
    while at<end {
        let (entry_start,entry_kind,rowid,id):(u64,i64,i64,String)=conn.query_row("SELECT record_start,entry_kind,native_rowid,entry_id FROM native_db_entries
            WHERE source_id=?1 AND generation=?2 AND record_start<=?3 AND record_end>?3 ORDER BY record_start DESC LIMIT 1",
            params![source.id,source.generation,at],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?))).map_err(sql)?;
        let kind=if entry_kind==0 {Entry::OpenCodeMessage} else {Entry::OpenCodePart};
        let selected:bool=native.query_row(&format!("SELECT EXISTS(SELECT 1 FROM {} WHERE rowid=?1 AND id=?2 AND session_id=?3)",kind.table()),
            params![rowid,id,source.segment.native_id],|row|row.get(0)).map_err(sql)?;
        if !selected {return Err("source_changed: native database entry identity changed".into());}
        let (from,to,hash):(u64,u64,Vec<u8>)=conn.query_row("SELECT start,end,hash FROM source_blocks WHERE source_id=?1 AND generation=?2 AND start<=?3 AND end>?3 ORDER BY start DESC LIMIT 1",
            params![source.id,source.generation,at],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).map_err(sql)?;
        if to<from || to-from>INPUT_BYTES as u64 || from<entry_start {return Err("detail_unavailable: native database verification range is invalid".into());}
        let mut blob=native.blob_open(DatabaseName::Main,kind.table(),kind.column(),rowid,true).map_err(|_|"source_changed: native database entry is unavailable")?;
        blob.seek(SeekFrom::Start(from-entry_start)).map_err(|_|"source_changed: native database field is unavailable")?;
        let mut bytes=vec![0;(to-from) as usize];blob.read_exact(&mut bytes).map_err(|_|"source_changed: native database entry was truncated")?;
        if Sha256::digest(&bytes).as_slice()!=hash {return Err("source_changed: native database field changed".into());}
        let next=to.min(end);output.extend_from_slice(&bytes[(at-from) as usize..(next-from) as usize]);at=next;
    }
    if expected.as_deref()!=std::fs::metadata(&source.segment.path).ok().as_ref().map(source::identity).as_deref() {
        return Err("source_changed: native database was replaced during the read".into());
    }Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::api::Branch;
    fn database(temp:&super::super::tests::Temp) -> (PathBuf,Connection) {
        let path=temp.0.join("native.sqlite");let conn=Connection::open(&path).unwrap();
        conn.execute_batch("CREATE TABLE session(id TEXT PRIMARY KEY,revert TEXT);
            CREATE TABLE message(id TEXT PRIMARY KEY,session_id TEXT,time_created INTEGER,time_updated INTEGER,data TEXT);
            CREATE TABLE part(id TEXT PRIMARY KEY,session_id TEXT,message_id TEXT,time_created INTEGER,time_updated INTEGER,data TEXT);
            INSERT INTO session VALUES('selected',NULL);INSERT INTO session VALUES('foreign',NULL);").unwrap();(path,conn)
    }
    fn message(conn:&Connection,id:&str,session:&str,time:i64,role:&str) {
        conn.execute("INSERT INTO message VALUES(?1,?2,?3,?3,?4)",params![id,session,time,json!({"role":role,"time":{"created":time}}).to_string()]).unwrap();
    }
    fn part(conn:&Connection,id:&str,message:&str,session:&str,time:i64,value:Value) {
        conn.execute("INSERT INTO part VALUES(?1,?2,?3,?4,?4,?5)",params![id,session,message,time,value.to_string()]).unwrap();
    }
    fn branch(temp:&super::super::tests::Temp,path:&Path,session:&str) -> Branch {
        let store=Store::at(&temp.0).unwrap();let source=register(&store,path.into(),session).unwrap();scan(&store,&source,&AtomicBool::new(false)).unwrap();
        let conn=store.connect().unwrap();conn.execute("INSERT INTO segments VALUES(?1,0,?2,?3,NULL,NULL)",params![session,source.id,source.generation]).unwrap();
        Branch {store,sources:vec![source],generation:"fixture".into(),kind:SessionKind::Opencode,native_id:session.into(),error:None,scope:session.into(),parent:None}
    }
    fn detail(branch:&Branch,reference:&str) -> String {
        let mut text=String::new();let mut sequence=0;loop {let part=branch.detail(reference,None,sequence).unwrap();text.push_str(part["text"].as_str().unwrap());
            assert!(part.to_string().len()<=DETAIL_BYTES);if part["eof"]==true {break;}sequence+=1;}text
    }

    #[test]
    fn native_database_pages_and_full_tool_fields_do_not_copy_or_materialize_long_columns() {
        let temp=super::super::tests::Temp::new();let (path,conn)=database(&temp);let text="雪😀 native original\n".repeat(90_000);
        message(&conn,"user","selected",0,"user");part(&conn,"user-one","user","selected",0,json!({"type":"text","text":"First original"}));
        part(&conn,"user-two","user","selected",1,json!({"type":"text","text":"Second original"}));
        for number in 1..75 {let id=format!("message-{number:03}");message(&conn,&id,"selected",number,"assistant");
            part(&conn,&format!("part-{number:03}"),&id,"selected",number,json!({"type":"text","text":format!("Answer {number}")}));}
        message(&conn,"tool-message","selected",100,"assistant");part(&conn,"long-tool","tool-message","selected",100,
            json!({"type":"tool","tool":"write","callID":"call","state":{"status":"error","input":{"filePath":"file.txt","content":text,"lateNumber":719,"lateFlag":true},"error":"Failed original"}}));
        message(&conn,"foreign-message","foreign",0,"user");part(&conn,"foreign-part","foreign-message","foreign",0,json!({"type":"text","text":"Outside the selected scope"}));
        let before=std::fs::read(&path).unwrap();let branch=branch(&temp,&path,"selected");let page=branch.page(None,None,None).unwrap();
        assert_eq!(page["coverage"],"complete");assert_eq!(page["knownRows"],76);assert_eq!(page["rows"].as_array().unwrap().len(),60);
        assert!(page.to_string().len()<PAGE_BYTES);assert_eq!(page["rows"].as_array().unwrap().last().unwrap()["status"],"failed");
        let row=page["rows"].as_array().unwrap().last().unwrap();assert!(row["input"].to_string().len()<1024);let manifest=branch.manifest(row["id"].as_str().unwrap(),0).unwrap();
        let input=manifest["fields"].as_array().unwrap().iter().find(|field|field["path"]=="/part/state/input").unwrap();
        let input:Value=serde_json::from_str(&detail(&branch,input["detailRef"].as_str().unwrap())).unwrap();
        assert_eq!(input["content"],text);assert_eq!(input["lateNumber"],719);assert_eq!(input["lateFlag"],true);
        let first=branch.forward(None).unwrap()[0].1.clone();assert_eq!(first["text"],"First original\nSecond original");
        // A retained registration predates the scanner's commits. Reusing it must not append the
        // same native multipart user content a second time.
        scan(&branch.store,&branch.sources[0],&AtomicBool::new(false)).unwrap();
        assert_eq!(branch.forward(None).unwrap()[0].1,first);
        assert_eq!(std::fs::read(&path).unwrap(),before);assert!(std::fs::read_dir(temp.0.join("chat-index")).unwrap().all(|entry|entry.unwrap().path().extension().and_then(|extension|extension.to_str())!=Some("jsonl")));
        let foreign=register(&branch.store,path,"foreign").unwrap();assert_ne!(foreign.id,branch.sources[0].id);
        let foreign_branch=Branch {sources:vec![foreign],native_id:"foreign".into(),scope:"foreign".into(),..branch};
        assert!(foreign_branch.detail(manifest["fields"][0]["detailRef"].as_str().unwrap(),None,0).is_err());
    }

    #[test]
    fn native_child_sessions_require_each_parent_tool_and_keep_sibling_details_private() {
        let temp=super::super::tests::Temp::new();let (path,conn)=database(&temp);
        conn.execute("INSERT INTO session VALUES('child',NULL)",[]).unwrap();
        message(&conn,"parent","selected",1,"assistant");part(&conn,"task","parent","selected",1,json!({"type":"tool","tool":"task","callID":"spawn",
            "state":{"status":"completed","input":{"description":"Delegated check"},"output":"Done","metadata":{"sessionId":"child"}}}));
        for number in 0..161 {let id=format!("child-message-{number:03}");message(&conn,&id,"child",number,"assistant");
            part(&conn,&format!("child-part-{number:03}"),&id,"child",number,json!({"type":"text","text":format!("Child answer {number}")}));}
        message(&conn,"foreign-message","foreign",1,"assistant");part(&conn,"foreign-part","foreign-message","foreign",1,json!({"type":"text","text":"Private sibling"}));
        let before=std::fs::read(&path).unwrap();let root=branch(&temp,&path,"selected");let page=root.page(None,None,None).unwrap();
        assert_eq!(page["knownRows"],1);assert_eq!(page["rows"][0]["childAvailable"],true);let id=page["rows"][0]["id"].as_str().unwrap().to_owned();
        assert_eq!(root.store.connect().unwrap().query_row("SELECT count(*) FROM sources",[],|row|row.get::<_,u64>(0)).unwrap(),1);
        assert!(super::super::children::descend(root.clone(),&["child".into()]).is_err());
        let child=super::super::children::descend(root.clone(),&[id]).unwrap();scan(&child.store,&child.sources[0],&AtomicBool::new(false)).unwrap();
        let page=child.page(None,None,None).unwrap();assert_eq!(page["knownRows"],161);assert_eq!(page["rows"].as_array().unwrap().len(),60);assert_eq!(page["hasMore"],true);
        let manifest=child.manifest(page["rows"][0]["id"].as_str().unwrap(),0).unwrap();let reference=manifest["fields"][0]["detailRef"].as_str().unwrap();
        assert!(root.detail(reference,None,0).is_err());assert!(detail(&child,reference).starts_with("Child answer"));
        assert!(!page.to_string().contains("Private sibling"));assert_eq!(std::fs::read(&path).unwrap(),before);
    }

    #[test]
    fn native_database_append_preserves_identity_and_revert_or_replacement_invalidates_old_ranges() {
        let temp=super::super::tests::Temp::new();let (path,conn)=database(&temp);
        message(&conn,"one","selected",1,"assistant");part(&conn,"one-part","one","selected",1,json!({"type":"text","text":"Original answer"}));
        let mut branch=branch(&temp,&path,"selected");let first=branch.page(None,None,None).unwrap()["rows"][0].clone();
        let reference=branch.manifest(first["id"].as_str().unwrap(),0).unwrap()["fields"][0]["detailRef"].as_str().unwrap().to_owned();
        message(&conn,"two","selected",2,"user");part(&conn,"two-part","two","selected",2,json!({"type":"text","text":"New question"}));
        let appended=register(&branch.store,path.clone(),"selected").unwrap();assert_eq!(appended.generation,branch.sources[0].generation);
        scan(&branch.store,&appended,&AtomicBool::new(false)).unwrap();branch.sources=vec![appended];assert_eq!(detail(&branch,&reference),"Original answer");assert_eq!(branch.page(None,None,None).unwrap()["knownRows"],2);
        conn.execute("UPDATE session SET revert=?1 WHERE id='selected'",["{\"messageID\":\"two\"}"]).unwrap();
        let reverted=register(&branch.store,path.clone(),"selected").unwrap();assert_ne!(reverted.generation,branch.sources[0].generation);
        scan(&branch.store,&reverted,&AtomicBool::new(false)).unwrap();branch.sources=vec![reverted];assert_eq!(branch.page(None,None,None).unwrap()["knownRows"],1);
        assert!(branch.detail(&reference,None,0).unwrap_err().starts_with("source_changed:"));
        let row=branch.page(None,None,None).unwrap()["rows"][0].clone();let reference=branch.manifest(row["id"].as_str().unwrap(),0).unwrap()["fields"][0]["detailRef"].as_str().unwrap().to_owned();
        conn.execute("UPDATE part SET data=?1 WHERE id='one-part'",[json!({"type":"text","text":"Replaced answer"}).to_string()]).unwrap();
        assert!(branch.detail(&reference,None,0).unwrap_err().starts_with("source_changed:"));
    }

    #[test]
    fn native_multipart_prose_shell_markers_and_export_match_existing_semantics() {
        use crate::agent::opencode_store::OpencodeMessage;
        let temp=super::super::tests::Temp::new();let (path,conn)=database(&temp);let mut messages=Vec::new();
        let fixtures=[
            ("user",1,json!({"role":"user","time":{"created":1000}}),vec![json!({"type":"text","text":" First "}),json!({"type":"text","text":" Second "}),json!({"type":"file","mime":"image/png","url":"data:image/png;base64,AA=="})]),
            ("answer",2,json!({"role":"assistant","time":{"created":2000}}),vec![json!({"type":"text","text":" Answer "}),json!({"type":"tool","tool":"write","callID":"write-call","state":{"status":"completed","input":{"filePath":"file.txt","content":"Small original"},"output":"Written"}})]),
            ("shell",3,json!({"role":"user","time":{"created":3000}}),vec![json!({"type":"text","synthetic":true,"text":"The following tool was executed by the user"})]),
            ("shell-result",4,json!({"role":"assistant","parentID":"shell","time":{"created":4000}}),vec![json!({"type":"tool","tool":"bash","callID":"shell-call","state":{"status":"completed","input":{"command":"pwd"},"output":"/repo"}})]),
            ("injected",5,json!({"role":"user","time":{"created":5000}}),vec![json!({"type":"text","synthetic":true,"text":"Internal injected context"})]),
            ("unrelated",6,json!({"role":"assistant","parentID":"another-user","time":{"created":6000}}),vec![json!({"type":"tool","tool":"bash","callID":"other-call","state":{"status":"completed","input":{"command":"wrong"},"output":"other"}})]),
        ];
        for (id,time,mut info,mut parts) in fixtures {
            conn.execute("INSERT INTO message VALUES(?1,'selected',?2,?2,?3)",params![id,time,info.to_string()]).unwrap();info["id"]=json!(id);
            for (number,part_value) in parts.iter_mut().enumerate() {
                let part_id=format!("{id}-{number}");part(&conn,&part_id,id,"selected",time,part_value.clone());part_value["id"]=json!(part_id);
            }messages.push(OpencodeMessage {info,parts});
        }
        let branch=branch(&temp,&path,"selected");let cancel=AtomicBool::new(false);
        let first=branch.forward(None).unwrap()[0].1.clone();let manifest=branch.manifest(first["id"].as_str().unwrap(),0).unwrap();
        assert_eq!(manifest["fields"].as_array().unwrap().iter().filter(|field|field["label"]=="body").count(),3);
        assert_eq!(detail(&branch,manifest["fields"][0]["detailRef"].as_str().unwrap())," First ");
        assert_eq!(detail(&branch,manifest["fields"][1]["detailRef"].as_str().unwrap())," Second ");
        let events=crate::agent::chat::opencode_timeline::events(&messages);
        let expected=crate::agent::export::render(&events,"Fixture","opencode","selected",&path.display().to_string(),None);
        let mut exported=Vec::new();super::super::markdown::write_markdown(branch.clone(),"Fixture",None,&cancel,&mut exported).unwrap();
        assert_eq!(String::from_utf8(exported).unwrap(),expected);
        let prose=super::super::prose::Prose::new(branch.clone(),&cancel).unwrap();
        let all=prose.collect_bounded(0,prose.total,512*1024,&cancel).unwrap();
        assert_eq!(all[0].text," First \n Second \n[image]");assert_eq!(all[1].text," Answer ");assert_eq!(all[1].tools,["Write"]);
        assert_eq!(all[2].text,"!pwd");assert_eq!(all[3].tools,["Bash"]);assert_eq!(prose.total,4);
        assert_eq!(all[3].text,"");
        let rows=branch.forward(None).unwrap();assert!(rows.iter().any(|(_,row)|row["kind"]=="user" && row["text"]=="!pwd"));
        assert!(!rows.iter().any(|(_,row)|row["kind"]=="user" && row["text"]=="!wrong"));
    }

    #[test]
    fn native_long_user_message_and_unknown_tool_metadata_remain_readable_and_cancellable() {
        let temp=super::super::tests::Temp::new();let (path,conn)=database(&temp);let text="雪😀 original user\n".repeat(20_000);
        message(&conn,"user","selected",1,"user");part(&conn,"first","user","selected",1,json!({"type":"text","text":"First"}));
        part(&conn,"long","user","selected",2,json!({"type":"text","text":text}));
        message(&conn,"answer","selected",2,"assistant");part(&conn,"tool","answer","selected",3,json!({"type":"tool","tool":"write","callID":"call",
            "state":{"status":"error","input":{"filePath":"file.txt","content":text,"lateNumber":719},"metadata":{"unrecognized":"Original metadata","diff":text},"error":"Failed"}}));
        let branch=branch(&temp,&path,"selected");let cancel=AtomicBool::new(false);
        let prose=super::super::prose::Prose::new(branch.clone(),&cancel).unwrap();let all=prose.collect_bounded(0,2,2*1024*1024,&cancel).unwrap();
        assert_eq!(all[0].text,format!("First\n{text}"));assert_eq!(all[1].tools,["Write"]);
        let rows=branch.forward(None).unwrap();let tool=&rows.last().unwrap().1;let manifest=branch.manifest(tool["id"].as_str().unwrap(),0).unwrap();
        let original=manifest["fields"].as_array().unwrap().iter().find(|field|field["label"]=="original").unwrap();
        let part:Value=serde_json::from_str(&detail(&branch,original["detailRef"].as_str().unwrap())).unwrap();
        assert_eq!(part["state"]["metadata"]["diff"],text);assert_eq!(part["state"]["input"]["lateNumber"],719);
        let mut exported=Vec::new();super::super::markdown::write_markdown(branch,"Fixture",None,&cancel,&mut exported).unwrap();
        let exported=String::from_utf8(exported).unwrap();assert_eq!(exported.matches("original user").count(),40_000);assert!(exported.contains("719"));
        cancel.store(true,Ordering::Relaxed);assert_eq!(prose.collect_bounded(0,1,2*1024*1024,&cancel).unwrap_err(),"read_canceled");
    }

}

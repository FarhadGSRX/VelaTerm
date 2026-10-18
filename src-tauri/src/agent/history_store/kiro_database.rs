//! Kiro's verified SQLite text format, split into bounded native history elements. Native files are
//! never opened by SQLite: only a generation-checked private DB/WAL copy can create a sidecar.

use std::collections::{HashMap,HashSet};
use std::io::{Read,Seek,SeekFrom};
use std::path::{Path,PathBuf};
use std::sync::{Arc,Mutex,OnceLock,Weak};
use std::sync::atomic::{AtomicBool,Ordering};
use std::time::{Duration,Instant};
use rusqlite::{params,Connection,DatabaseName,OptionalExtension};
use serde_json::{json,Value};
use sha2::{Digest,Sha256};
use crate::agent::kiro_store;
use super::{adapter,limits::*,native_database,projection::{Checkpoint,Projector,Record},source::{self,Segment},store::{Store,RegisteredSource,sql,digest}};

#[derive(Clone)]
pub(super) struct Source {modified:String,store:Weak<Store>}
impl std::fmt::Debug for Source {
    fn fmt(&self,f:&mut std::fmt::Formatter<'_>) -> std::fmt::Result {f.debug_struct("KiroSource").field("modified",&self.modified).finish()}
}
struct Reservation {store:Weak<Store>,path:PathBuf}
impl Drop for Reservation {fn drop(&mut self) {if let Some(store)=self.store.upgrade() {store.release_scratch(&self.path);}}}
struct Copy {path:PathBuf,owner:PathBuf,_directory:Arc<kiro_store::snapshot::TempDir>,_reservation:Reservation}
struct Copies {ready:HashMap<String,(Arc<Copy>,Instant)>,pending:HashSet<String>}
static COPIES:OnceLock<Mutex<Copies>>=OnceLock::new();

fn physical_bytes(path:&Path) -> Result<u64,String> {
    let mut bytes=0u64;
    for path in [path.to_owned(),PathBuf::from(format!("{}-wal",path.to_string_lossy()))] {
        match std::fs::symlink_metadata(path) {
            Ok(metadata) if metadata.is_file()=>bytes=bytes.checked_add(metadata.len()).ok_or("unsupported_record: Kiro database byte count overflow")?,
            Ok(_)=>return Err("source_missing: Kiro history source must be a regular file".into()),
            Err(error) if error.kind()==std::io::ErrorKind::NotFound=>{},
            Err(_)=>return Err("source_missing: Kiro history metadata is unavailable".into()),
        }
    }Ok(bytes)
}

pub(super) fn register(store:&Arc<Store>,path:PathBuf,native:&str) -> Result<RegisteredSource,String> {
    if !kiro_store::valid_id(native) {return Err("unsupported_record: Kiro conversation identity is invalid".into());}
    let metadata=std::fs::symlink_metadata(&path).map_err(|_|"source_missing: Kiro database is unavailable")?;
    if !metadata.is_file() {return Err("source_missing: Kiro database must be a regular file".into());}
    let identity=source::identity(&metadata);let modified=native_database::stamp(&path)?;let size=physical_bytes(&path)?;
    let _gate=store.cache_gate.lock().unwrap();let conn=store.connect()?;let key=format!("sqlite:kiro:{}:{native}",path.to_string_lossy());
    let previous:Option<(String,String,String,String,u32,u64,String)>=conn.query_row("SELECT id,generation,identity,modified,parser_version,indexed_bytes,status
        FROM sources WHERE path=?1",[&key],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?,row.get(6)?))).optional().map_err(sql)?;
    let (id,generation,indexed,status)=match previous {
        Some((id,generation,old_identity,old_modified,version,indexed,status)) if old_identity==identity && old_modified==modified && version==PARSER_VERSION=>(id,generation,indexed,status),
        Some((id,..))=>(id,uuid::Uuid::new_v4().to_string(),0,"partial".into()),
        None=>(uuid::Uuid::new_v4().to_string(),uuid::Uuid::new_v4().to_string(),0,"partial".into()),
    };
    conn.execute("INSERT INTO sources(id,provider,native_id,path,identity,generation,size,modified,head_hash,indexed_bytes,status,parser_version)
        VALUES(?1,'kiro',?2,?3,?4,?5,?6,?7,X'',?8,?9,?10) ON CONFLICT(path) DO UPDATE SET generation=excluded.generation,identity=excluded.identity,
            size=excluded.size,modified=excluded.modified,indexed_bytes=excluded.indexed_bytes,status=excluded.status,parser_version=excluded.parser_version,error=NULL",
        params![id,native,key,identity,generation,size,modified,indexed,status,PARSER_VERSION]).map_err(sql)?;
    conn.execute("INSERT INTO generation_access VALUES(?1,?2,?3) ON CONFLICT(source_id,generation) DO UPDATE SET touched=excluded.touched",
        params![id,generation,super::cache::now()]).map_err(sql)?;
    for (key,cancel) in store.jobs.lock().unwrap().iter() {if key.starts_with(&format!("{id}:")) && key!=&format!("{id}:{generation}") {cancel.store(true,Ordering::Relaxed);}}
    let _pin=super::cache::Pin::new(store,format!("{id}:{generation}"));super::cache::trim_locked(store,&conn)?;
    Ok(RegisteredSource {id,generation,segment:Segment {path,native_id:native.into(),end_byte:None,end_ordinal:None},size,indexed,status,database:None,
        kiro:Some(Source {modified,store:Arc::downgrade(store)}),kiro_pair:None,_pin})
}

fn copy(source:&RegisteredSource,cancel:&AtomicBool) -> Result<Arc<Copy>,String> {
    if cancel.load(Ordering::Relaxed) {return Err("read_canceled".into());}
    let descriptor=source.kiro.as_ref().ok_or("unsupported_record: Kiro source descriptor is unavailable")?;
    if native_database::stamp(&source.segment.path)?!=descriptor.modified {return Err("source_changed: Kiro database changed".into());}
    let store=descriptor.store.upgrade().ok_or("cursor_expired: Kiro history reader expired")?;
    let key=digest(format!("{}:{}:{}",store.path.display(),source.segment.path.display(),descriptor.modified).as_bytes());
    let copies=COPIES.get_or_init(||Mutex::new(Copies {ready:HashMap::new(),pending:HashSet::new()}));
    {
        let mut cache=copies.lock().unwrap();
        cache.ready.retain(|_,(copy,touched)|Arc::strong_count(copy)>1 || touched.elapsed()<Duration::from_secs(600));
        if let Some((copy,touched))=cache.ready.get_mut(&key) {*touched=Instant::now();return Ok(copy.clone());}
        if cache.pending.contains(&key) {return Err("index_busy: Kiro snapshot is being prepared by another reader".into());}
        if cache.ready.len()+cache.pending.len()>=4 {
            let oldest=cache.ready.iter().filter(|(_,item)|Arc::strong_count(&item.0)==1).min_by_key(|(_,item)|item.1).map(|(key,_)|key.clone());
            if let Some(oldest)=oldest {cache.ready.remove(&oldest);}
            else {return Err("index_busy: all native database snapshots are in use".into());}
        }
        cache.pending.insert(key.clone());
    }
    let result=(|| {
        let parent=store.path.parent().unwrap().join("native-snapshots");
        let mut builder=std::fs::DirBuilder::new();builder.recursive(true);
        #[cfg(unix)] {use std::os::unix::fs::DirBuilderExt;builder.mode(0o700);}
        builder.create(&parent).map_err(|_|"storage_pressure: native snapshot storage is unavailable")?;
        if !std::fs::symlink_metadata(&parent).is_ok_and(|metadata|metadata.is_dir()) {return Err("storage_pressure: native snapshot storage must be a directory".into());}
        let mut options=std::fs::OpenOptions::new();options.create(true).truncate(false).read(true).write(true);
        #[cfg(unix)] {use std::os::unix::fs::OpenOptionsExt;options.mode(0o600);}
        let quota=options.open(parent.join("budget.lock")).map_err(|_|"storage_pressure: native snapshot budget ownership is unavailable")?;
        quota.try_lock().map_err(|_|"index_busy: another process is reserving native snapshot storage")?;
        let directory=Arc::new(kiro_store::snapshot::TempDir::in_parent(Some(&parent))?);let reservation_path=directory.0.clone();
        // Reserve DB, WAL and the bounded private sidecar allowance before copying any source bytes.
        let sidecar=super::cache::WAL_RESERVE;
        let bytes=physical_bytes(&source.segment.path)?;
        loop {
            match store.reserve_scratch(&reservation_path,bytes.saturating_add(sidecar)) {
                Ok(())=>break,
                Err(error) if error.starts_with("storage_pressure:")=> {
                    let evicted={let mut cache=copies.lock().unwrap();
                        let oldest=cache.ready.iter().filter(|(_,item)|Arc::strong_count(&item.0)==1
                            && item.0.owner==store.path)
                            .min_by_key(|(_,item)|item.1).map(|(key,_)|key.clone());
                        oldest.and_then(|key|cache.ready.remove(&key))};
                    if evicted.is_none() {return Err(error);}drop(evicted);
                },
                Err(error)=>return Err(error),
            }
        }
        let reservation=Reservation {store:Arc::downgrade(&store),path:reservation_path};
        directory.reserve(bytes.saturating_add(sidecar))?;drop(quota);
        let db=kiro_store::snapshot::database_reserved(&source.segment.path,directory,bytes,cancel)?;
        if cancel.load(Ordering::Relaxed) {return Err("read_canceled".into());}
        if native_database::stamp(&source.segment.path)?!=descriptor.modified {return Err("source_changed: Kiro database changed while preparing its snapshot".into());}
        Ok(Arc::new(Copy {path:db.copy.clone(),owner:store.path.clone(),_directory:db.retain(),_reservation:reservation}))
    })();
    let mut cache=copies.lock().unwrap();cache.pending.remove(&key);
    if let Ok(copy)=&result {cache.ready.insert(key,(copy.clone(),Instant::now()));}result
}

pub(super) fn has_session(source:&RegisteredSource,cancel:&AtomicBool) -> Result<bool,String> {
    let private=copy(source,cancel)?;let conn=super::sqlite_source::open(&private.path)?;
    conn.query_row("SELECT EXISTS(SELECT 1 FROM conversations_v2 WHERE conversation_id=?1)",[&source.segment.native_id],|row|row.get(0)).map_err(sql)
}

struct Input<'a> {blob:rusqlite::blob::Blob<'a>,conn:&'a Connection,source:&'a RegisteredSource,cancel:&'a AtomicBool,offset:u64}
impl Read for Input<'_> {
    fn read(&mut self,bytes:&mut [u8]) -> std::io::Result<usize> {
        if self.cancel.load(Ordering::Relaxed) {return Err(std::io::Error::other("read_canceled"));}
        let count=self.blob.read(bytes)?;
        if count>0 {
            let hash=Sha256::digest(&bytes[..count]).to_vec();
            let previous:Option<(u64,Vec<u8>)>=self.conn.query_row("SELECT end,hash FROM source_blocks WHERE source_id=?1 AND generation=?2 AND start=?3",
                params![self.source.id,self.source.generation,self.offset],|row|Ok((row.get(0)?,row.get(1)?))).optional().map_err(std::io::Error::other)?;
            if previous.is_some_and(|(end,old)|end==self.offset+count as u64 && hash!=old) {return Err(std::io::Error::other("source_changed: Kiro indexed body changed"));}
            self.conn.execute("INSERT OR REPLACE INTO source_blocks VALUES(?1,?2,?3,?4,?5)",params![self.source.id,self.source.generation,self.offset,self.offset+count as u64,hash])
                .map_err(std::io::Error::other)?;self.offset+=count as u64;
        }Ok(count)
    }
}
fn checkpoint(conn:&Connection,source:&RegisteredSource,piece:Checkpoint) -> Result<(),String> {
    conn.execute("INSERT OR REPLACE INTO checkpoints VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",params![source.id,source.generation,piece.record_start,
        piece.path,piece.raw_start,piece.raw_end,piece.decoded_start,piece.decoded_end,piece.hash,piece.sequence]).map_err(sql)?;Ok(())
}

fn element(conn:&Connection,source:&RegisteredSource,record:&mut Record,cwd:&Path) -> Result<(),String> {
    let row=&record.value;let prompt=row.pointer("/user/content/Prompt/prompt").and_then(Value::as_str)
        .ok_or("unsupported_record: Kiro history contains a non-text user event")?;
    let assistant=row.pointer("/assistant/Response").ok_or("unsupported_record: Kiro history contains a non-text assistant event")?;
    let id=assistant["message_id"].as_str().filter(|id|kiro_store::valid_id(id)).ok_or("unsupported_record: Kiro assistant identity is invalid")?;
    let answer=assistant["content"].as_str().ok_or("unsupported_record: Kiro assistant content is invalid")?;
    for path in ["/request_metadata/tool_use_ids_and_names","/user/images"] {
        if row.pointer(path).is_some_and(|value|!value.is_null() && value.as_array().is_none_or(|array|!array.is_empty())) {
            return Err("unsupported_record: Kiro history contains tool activity or images that require a verified adapter".into());
        }
    }
    if let Some(directory)=row.pointer("/user/env_context/env_state/current_working_directory").filter(|value|!value.is_null()) {
        let directory=kiro_store::parse::directory(directory.as_str().ok_or("unsupported_record: Kiro message directory is invalid")?)?;
        if directory.canonicalize().unwrap_or(directory)!=cwd.canonicalize().unwrap_or_else(|_|cwd.to_owned()) {return Err("source_changed: Kiro message project identity conflicts with its database".into());}
    }
    let timestamp=kiro_store::parse::iso_optional(&row["user"]["timestamp"])?;
    let context=kiro_store::parse::context(&Value::Null,&row["request_metadata"]["context_usage_percentage"])?;
    let duplicate:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM records WHERE source_id=?1 AND generation=?2 AND native_id=?3 AND start!=?4)",
        params![source.id,source.generation,id,record.start],|row|row.get(0)).map_err(sql)?;
    if duplicate {return Err("unsupported_record: duplicate Kiro assistant message identity".into());}
    let refs=adapter::store_fields(conn,source,record)?;
    conn.execute("INSERT OR REPLACE INTO records(source_id,generation,start,end,native_id,event_type,projection,active) VALUES(?1,?2,?3,?4,?5,'kiro_pair',?6,0)",
        params![source.id,source.generation,record.start,record.end,id,json!({"type":"kiro_pair","id":id,"prompt":prompt,"answer":answer,"timestamp":timestamp,"percent":context.percent}).to_string()]).map_err(sql)?;
    for (local,kind,text,path,at) in [(0,"user",prompt,"/user/content/Prompt/prompt",timestamp.as_deref()),(1,"assistant",answer,"/assistant/Response/content",None)] {
        let stable=format!("native:{}",digest(format!("kiro:{}:{id}:{kind}",source.segment.native_id).as_bytes()));
        let field=record.fields.iter().find(|field|field.path==path && field.encoding=="string").ok_or("unsupported_record: Kiro body range is unavailable")?;
        let mut row=json!({"kind":kind,"id":stable,"text":adapter::preview(text,PREVIEW_BYTES),"streaming":false,"bodySizeBytes":field.decoded_bytes});
        if field.truncated {row["bodyAvailable"]=json!(true);}
        if let Some(at)=crate::agent::chat::history::parsed_at(at) {row["at"]=json!(at);}
        adapter::write_row(conn,source,record,local,None,&row)?;adapter::attach(conn,source,&stable,"body",&[path.into()],&refs)?;
    }Ok(())
}

pub(super) fn scan(store:&Store,source:&RegisteredSource,cancel:&AtomicBool) -> Result<(),String> {
    let private=copy(source,cancel)?;let native=super::sqlite_source::open(&private.path)?;let conn=store.connect()?;
    let mut statement=native.prepare("SELECT rowid,CASE WHEN octet_length(key)<=8192 THEN key END,created_at,updated_at,octet_length(value)
        FROM conversations_v2 WHERE conversation_id=?1 LIMIT 2").map_err(|_|"unsupported_record: Kiro database schema is unsupported")?;
    let mut rows=statement.query([&source.segment.native_id]).map_err(sql)?;
    let row=rows.next().map_err(sql)?.ok_or("source_missing: Kiro conversation is unavailable")?;
    let (rowid,key,created,updated,size):(i64,Option<String>,i64,i64,u64)=(row.get(0).map_err(sql)?,row.get(1).map_err(sql)?,row.get(2).map_err(sql)?,row.get(3).map_err(sql)?,row.get(4).map_err(sql)?);
    if rows.next().map_err(sql)?.is_some() {return Err("source_changed: Kiro history has conflicting database identities".into());}
    if updated<created {return Err("unsupported_record: Kiro database time order is invalid".into());}
    let cwd=kiro_store::parse::directory(&key.ok_or("unsupported_record: Kiro project directory exceeds its metadata budget")?)?;
    let blob=native.blob_open(DatabaseName::Main,"conversations_v2","value",rowid,true).map_err(|_|"unsupported_record: Kiro history column cannot be read incrementally")?;
    let mut parser=Projector::document(Input {blob,conn:&conn,source,cancel,offset:0},0);let mut checkpoints=0;
    conn.execute_batch("BEGIN IMMEDIATE").map_err(sql)?;
    let result=(|| {
        let header=parser.split_object_array("history",&mut |piece| {
            checkpoint(&conn,source,piece)?;checkpoints+=1;
            if checkpoints>=128 {conn.execute_batch("COMMIT").map_err(sql)?;super::cache::trim(store,&conn)?;super::cache::check_wal(store)?;
                conn.execute_batch("BEGIN IMMEDIATE").map_err(sql)?;checkpoints=0;std::thread::yield_now();}Ok(())
        },&mut |mut record| {
            if cancel.load(Ordering::Relaxed) {return Err("read_canceled".into());}
            let current:String=conn.query_row("SELECT generation FROM sources WHERE id=?1",[&source.id],|row|row.get(0)).map_err(sql)?;
            if current!=source.generation {return Err("source_changed: Kiro history generation changed".into());}
            element(&conn,source,&mut record,&cwd)?;
            conn.execute("UPDATE sources SET indexed_bytes=max(indexed_bytes,?3) WHERE id=?1 AND generation=?2",params![source.id,source.generation,record.end]).map_err(sql)?;
            conn.execute_batch("COMMIT").map_err(sql)?;super::cache::check_wal(store)?;conn.execute_batch("BEGIN IMMEDIATE").map_err(sql)?;Ok(())
        })?;
        if header.end!=size || header.value["conversation_id"]!=source.segment.native_id {return Err("source_changed: Kiro database and conversation identities conflict".into());}
        if !header.value["next_message"].is_null() || !header.value["latest_summary"].is_null() {return Err("unsupported_record: Kiro history contains pending or compacted content".into());}
        kiro_store::parse::context(&header.value["model_info"],&Value::Null)?;
        if native_database::stamp(&source.segment.path)?!=source.kiro.as_ref().unwrap().modified {return Err("source_changed: Kiro database changed while scanning".into());}
        let current:String=conn.query_row("SELECT generation FROM sources WHERE id=?1",[&source.id],|row|row.get(0)).map_err(sql)?;
        if current!=source.generation {return Err("source_changed: Kiro history generation changed".into());}
        conn.execute("INSERT OR REPLACE INTO native_db_entries VALUES(?1,?2,0,?3,2,?4,?5,?5,?3)",params![source.id,source.generation,size,rowid,source.segment.native_id]).map_err(sql)?;
        let percent:Option<f64>=conn.query_row("SELECT json_extract(projection,'$.percent') FROM records WHERE source_id=?1 AND generation=?2 AND event_type='kiro_pair' ORDER BY start DESC LIMIT 1",
            params![source.id,source.generation],|row|row.get(0)).optional().map_err(sql)?.flatten();
        conn.execute("INSERT OR REPLACE INTO records(source_id,generation,start,end,event_type,projection,active) VALUES(?1,?2,0,?3,'kiro_metadata',?4,0)",
            params![source.id,source.generation,size,json!({"cwd":cwd,"updatedAt":updated,"model":header.value["model_info"],"percent":percent}).to_string()]).map_err(sql)?;
        conn.execute("UPDATE records SET active=1 WHERE source_id=?1 AND generation=?2",params![source.id,source.generation]).map_err(sql)?;
        conn.execute("UPDATE sources SET indexed_bytes=size,status='complete',error=NULL WHERE id=?1 AND generation=?2",params![source.id,source.generation]).map_err(sql)?;
        conn.execute_batch("COMMIT; PRAGMA wal_checkpoint(PASSIVE)").map_err(sql)?;Ok(())
    })();
    if result.is_err() {let _=conn.execute_batch("ROLLBACK");}result
}

pub(super) fn range(conn:&Connection,source:&RegisteredSource,start:u64,end:u64) -> Result<Vec<u8>,String> {
    if end<start || end-start>(CHECKPOINT_BYTES+16) as u64 {return Err("unsupported_record: Kiro detail exceeds its byte budget".into());}
    let private=copy(source,&AtomicBool::new(false))?;let native=super::sqlite_source::open(&private.path)?;
    let rowid:i64=conn.query_row("SELECT native_rowid FROM native_db_entries WHERE source_id=?1 AND generation=?2 AND entry_kind=2",
        params![source.id,source.generation],|row|row.get(0)).map_err(sql)?;
    let present:bool=native.query_row("SELECT EXISTS(SELECT 1 FROM conversations_v2 WHERE rowid=?1 AND conversation_id=?2)",params![rowid,source.segment.native_id],|row|row.get(0)).map_err(sql)?;
    if !present {return Err("source_changed: Kiro detail is outside its native conversation".into());}
    let mut blob=native.blob_open(DatabaseName::Main,"conversations_v2","value",rowid,true).map_err(|_|"source_changed: Kiro conversation is unavailable")?;
    let mut output=Vec::with_capacity((end-start) as usize);let mut at=start;
    while at<end {
        let (from,to,expected):(u64,u64,Vec<u8>)=conn.query_row("SELECT start,end,hash FROM source_blocks WHERE source_id=?1 AND generation=?2 AND start<=?3 AND end>?3 ORDER BY start DESC LIMIT 1",
            params![source.id,source.generation,at],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).map_err(sql)?;
        if to<from || to-from>INPUT_BYTES as u64 {return Err("unsupported_record: Kiro verification block exceeds its byte budget".into());}
        blob.seek(SeekFrom::Start(from)).map_err(|_|"cursor_expired: Kiro range is invalid")?;let mut bytes=vec![0;(to-from) as usize];
        blob.read_exact(&mut bytes).map_err(|_|"source_changed: Kiro detail was truncated")?;
        if Sha256::digest(&bytes).as_slice()!=expected {return Err("source_changed: Kiro original body changed".into());}
        let next=end.min(to);output.extend_from_slice(&bytes[(at-from) as usize..(next-from) as usize]);at=next;
    }
    if native_database::stamp(&source.segment.path)?!=source.kiro.as_ref().unwrap().modified {return Err("source_changed: Kiro database changed during the read".into());}Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::SessionKind;
    use super::super::api::Branch;

    fn branch(f:&kiro_store::tests::Fixture,id:&str) -> Branch {
        let store=Store::at(&f.dir).unwrap();let source=register(&store,f.roots.database.clone().unwrap(),id).unwrap();
        let conn=store.connect().unwrap();conn.execute("INSERT INTO segments VALUES(?1,0,?2,?3,NULL,NULL)",params![id,source.id,source.generation]).unwrap();
        Branch {store,sources:vec![source],generation:"fixture".into(),kind:SessionKind::Kiro,native_id:id.into(),scope:id.into(),error:None,parent:None}
    }
    fn original(branch:&Branch,reference:&str) -> String {
        let mut result=String::new();let mut sequence=0;
        loop {let part=branch.detail(reference,None,sequence).unwrap();assert!(part.to_string().len()<=DETAIL_BYTES);
            result.push_str(part["text"].as_str().unwrap());if part["eof"]==true {break;}sequence+=1;}result
    }

    #[test]
    fn kiro_pages_preserve_long_native_bodies_and_sidecars_without_materializing_the_history_array() {
        let f=kiro_store::tests::Fixture::new();let native=f.db();native.execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0;").unwrap();
        let long=" 雪😀 original Kiro body\n".repeat(9000);let mut value=kiro_store::tests::sqlite_value("selected",&long);
        let template=value["history"][0].clone();let mut history=Vec::new();
        for number in 0..120 {let mut element=template.clone();element["assistant"]["Response"]["message_id"]=json!(format!("answer-{number}"));
            if number>0 {element["user"]["content"]["Prompt"]["prompt"]=json!(format!("Prompt {number}"));}
            history.push(element);
        }value["history"]=json!(history);
        native.execute("INSERT INTO conversations_v2 VALUES(?1,'selected',?2,1,2)",params![f.dir.to_string_lossy(),value.to_string()]).unwrap();
        f.insert(&native,"foreign",&f.dir,"Foreign body");let path=f.roots.database.as_ref().unwrap();
        let paths=[path.clone(),PathBuf::from(format!("{}-wal",path.display())),PathBuf::from(format!("{}-shm",path.display()))];
        let before=paths.iter().map(|path|std::fs::read(path).unwrap()).collect::<Vec<_>>();
        let branch=branch(&f,"selected");assert_eq!(branch.store.scratch_bytes(),0);
        assert!(super::super::context::kiro_branch(&branch).unwrap_err().starts_with("index_busy:"));
        let cold=branch.page(None,None,None).unwrap();assert_eq!(cold["coverage"],"partial");assert_eq!(cold["indexing"],true);assert_eq!(cold["rows"],json!([]));
        scan(&branch.store,&branch.sources[0],&AtomicBool::new(false)).unwrap();
        let page=branch.page(None,None,None).unwrap();assert_eq!(page["coverage"],"complete");assert_eq!(page["knownRows"],240);assert_eq!(page["rows"].as_array().unwrap().len(),60);
        let context=super::super::context::kiro_branch(&branch).unwrap();assert_eq!(context.model.as_deref(),Some("native-model"));assert_eq!(context.percent,Some(12.5));
        assert!(page.to_string().len()<=PAGE_BYTES);let first=branch.forward(None).unwrap()[0].1.clone();assert!(first["text"].as_str().unwrap().len()<=PREVIEW_BYTES);
        assert_eq!(first["bodyAvailable"],true);let reference=branch.manifest(first["id"].as_str().unwrap(),0).unwrap()["fields"][0]["detailRef"].as_str().unwrap().to_owned();
        assert_eq!(original(&branch,&reference),long);let prose=super::super::prose::Prose::new(branch.clone(),&AtomicBool::new(false)).unwrap();
        assert_eq!(prose.total,240);let messages=prose.collect_bounded(0,2,512*1024,&AtomicBool::new(false)).unwrap();assert_eq!(messages[0].text,long);assert_eq!(messages[1].text,"Reply");
        scan(&branch.store,&branch.sources[0],&AtomicBool::new(false)).unwrap();assert_eq!(branch.forward(None).unwrap()[0].1,first);
        for (path,before) in paths.iter().zip(before) {assert_eq!(std::fs::read(path).unwrap(),before);}
        let foreign=register(&branch.store,path.clone(),"foreign").unwrap();let foreign=Branch {sources:vec![foreign],scope:"foreign".into(),native_id:"foreign".into(),..branch};
        assert!(foreign.detail(&reference,None,0).is_err());
    }

    #[test]
    fn kiro_identity_faults_and_unknown_content_never_publish_a_completed_prefix() {
        for mutation in ["identity","duplicate","directory","images","tools","pending","model","percent"] {
            let f=kiro_store::tests::Fixture::new();let native=f.db();let mut value=kiro_store::tests::sqlite_value("selected","Original");
            match mutation {
                "identity"=>value["conversation_id"]=json!("different"),
                "duplicate"=>{let duplicate=value["history"][0].clone();value["history"].as_array_mut().unwrap().push(duplicate);},
                "directory"=>value["history"][0]["user"]["env_context"]=json!({"env_state":{"current_working_directory":"/different"}}),
                "images"=>value["history"][0]["user"]["images"]=json!(["unsupported"]),
                "tools"=>value["history"][0]["request_metadata"]["tool_use_ids_and_names"]=json!(["unsupported"]),
                "pending"=>value["next_message"]=json!({"content":"Pending"}),
                "model"=>value["model_info"]["context_window_tokens"]=json!(0),
                "percent"=>value["history"][0]["request_metadata"]["context_usage_percentage"]=json!(101),
                _=>unreachable!(),
            }
            native.execute("INSERT INTO conversations_v2 VALUES(?1,'selected',?2,1,2)",params![f.dir.to_string_lossy(),value.to_string()]).unwrap();
            let branch=branch(&f,"selected");assert!(scan(&branch.store,&branch.sources[0],&AtomicBool::new(false)).is_err(),"{mutation}");
            assert_eq!(branch.page(None,None,None).unwrap()["rows"],json!([]),"{mutation}");
        }
    }

    #[test]
    fn kiro_changed_original_and_cancelled_reader_preserve_native_contents_and_reject_stale_details() {
        let f=kiro_store::tests::Fixture::new();let native=f.db();f.insert(&native,"selected",&f.dir,"Original");
        let branch=branch(&f,"selected");let canceled=AtomicBool::new(true);
        assert_eq!(scan(&branch.store,&branch.sources[0],&canceled).unwrap_err(),"read_canceled");assert_eq!(branch.store.scratch_bytes(),0);
        scan(&branch.store,&branch.sources[0],&AtomicBool::new(false)).unwrap();let first=branch.forward(None).unwrap()[0].1.clone();
        let reference=branch.manifest(first["id"].as_str().unwrap(),0).unwrap()["fields"][0]["detailRef"].as_str().unwrap().to_owned();
        assert_eq!(original(&branch,&reference),"Original");f.insert(&native,"selected",&f.dir,"Modified");
        assert!(branch.detail(&reference,None,0).unwrap_err().starts_with("source_changed:"));
        let fresh=register(&branch.store,f.roots.database.clone().unwrap(),"selected").unwrap();assert_ne!(fresh.generation,branch.sources[0].generation);
        scan(&branch.store,&fresh,&AtomicBool::new(false)).unwrap();let fresh=Branch {sources:vec![fresh],..branch};assert_eq!(fresh.forward(None).unwrap()[0].1["text"],"Modified");
    }
}

#[cfg(test)]
pub(super) fn release_test_directory(store_path:&Path) {
    if let Some(copies)=COPIES.get() {copies.lock().unwrap().ready.retain(|_,(copy,_)|copy.owner!=store_path);}
}

//! Session-authorized, byte-bounded history pages and immutable field chunks.

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::sync::Arc;

use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::db::repo;
use crate::host::AppCtx;
use crate::models::SessionKind;
use super::limits::*;
use super::projection::decode_checkpoint;
use super::source;
use super::store::{digest, sql, RegisteredSource, Store};

#[derive(Clone)]
pub struct Branch {
    pub store: Arc<Store>,
    pub sources: Vec<RegisteredSource>,
    pub generation: String,
    pub kind: SessionKind,
    pub native_id: String,
    pub error: Option<String>,
    pub scope: String,
    pub(super) parent:Option<Arc<super::children::Parent>>,
}

/// Native recording sizes captured before an agent process starts. Rows that process appends belong
/// to its live timeline, so history pages for the running conversation end at these offsets.
#[derive(Clone,Debug)]
pub struct ProcessCut {pub epoch:u64,pub sizes:Vec<(std::path::PathBuf,u64)>}

impl ProcessCut {
    /// Reads file metadata only; no recording byte is read before the agent starts. Database
    /// providers have no byte offset that separates earlier rows from rows the process will add.
    pub fn capture(kind:SessionKind,native_id:&str) -> Option<ProcessCut> {
        if matches!(kind,SessionKind::Opencode|SessionKind::Kiro) {return None;}
        let resolved=source::resolve(kind,native_id).ok()?;
        let sizes=resolved.segments.into_iter().map(|segment| {
            let size=std::fs::metadata(&segment.path).map(|meta|meta.len()).unwrap_or(0);(segment.path,size)
        }).collect();
        Some(ProcessCut {epoch:0,sizes})
    }
    pub fn has_history(&self) -> bool {self.sizes.iter().any(|(_,size)|*size>0)}
}

fn changed(conn:&Connection,source:&RegisteredSource) -> Result<(),String> {
    conn.execute("UPDATE sources SET generation=?2,indexed_bytes=0,status='partial' WHERE id=?1 AND generation=?3",
        params![source.id,uuid::Uuid::new_v4().to_string(),source.generation]).map_err(sql)?;
    Err("source_changed: history content changed".into())
}

/// Verify and return bytes from the same open source and the same input blocks. JSON containers use
/// these ranges instead of reconstructing a potentially enormous object from projected child fields.
pub(super) fn verified_range(conn:&Connection,source:&RegisteredSource,start:u64,end:u64) -> Result<Vec<u8>,String> {
    if end<start || end-start>(CHECKPOINT_BYTES+16) as u64 {return Err("History range exceeds its byte budget".into());}
    if source.database.is_some() || source.kiro.is_some() {
        let result=if source.kiro.is_some() {super::kiro_database::range(conn,source,start,end)} else {super::native_database::range(conn,source,start,end)};
        return match result {
            Err(error) if error.starts_with("source_changed:")=>{changed(conn,source)?;unreachable!()},
            result=>result,
        };
    }
    let mut file=File::open(&source.segment.path).map_err(|_|"detail_unavailable: history source is unavailable")?;
    let expected:String=conn.query_row("SELECT identity FROM sources WHERE id=?1 AND generation=?2",params![source.id,source.generation],|r|r.get(0)).map_err(sql)?;
    if source::identity(&file.metadata().map_err(|e|e.to_string())?)!=expected {changed(conn,source)?;}
    let mut result=Vec::with_capacity((end-start) as usize);
    let mut at=start;
    while at<end {
        let block:Option<(u64,u64,Vec<u8>)>=conn.query_row("SELECT start,end,hash FROM source_blocks WHERE source_id=?1 AND generation=?2
            AND start<=?3 AND end>?3 ORDER BY start DESC LIMIT 1",params![source.id,source.generation,at],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(sql)?;
        let (from,to,hash)=block.ok_or("index_incomplete: source block is not available")?;
        if to<from || to-from>INPUT_BYTES as u64 {return Err("History block exceeds its byte budget".into());}
        file.seek(SeekFrom::Start(from)).map_err(|e|e.to_string())?;
        let mut bytes=vec![0;(to-from) as usize];
        file.read_exact(&mut bytes).map_err(|_|"source_changed: history source was truncated")?;
        if Sha256::digest(&bytes).as_slice()!=hash {changed(conn,source)?;}
        let next=to.min(end);
        result.extend_from_slice(&bytes[(at-from) as usize..(next-from) as usize]); at=next;
    }
    let at_path=std::fs::metadata(&source.segment.path).map_err(|_|"source_changed: history source was removed")?;
    if source::identity(&at_path)!=expected || source::identity(&file.metadata().map_err(|e|e.to_string())?)!=expected {changed(conn,source)?;}
    Ok(result)
}

/// Read a raw range of the original file while its identity stays the indexed one. The caller
/// verifies the bytes against the checkpoint or field hash that covers exactly this range.
fn original_range(conn:&Connection,source:&RegisteredSource,raw_start:u64,raw_end:u64) -> Result<Vec<u8>,String> {
    let mut file=File::open(&source.segment.path).map_err(|_|"detail_unavailable: history source is unavailable")?;
    let expected_identity:String=conn.query_row("SELECT identity FROM sources WHERE id=?1 AND generation=?2",params![source.id,source.generation],|r|r.get(0)).map_err(sql)?;
    if source::identity(&file.metadata().map_err(|e|e.to_string())?)!=expected_identity {return Err("source_changed: history source was replaced".into());}
    file.seek(SeekFrom::Start(raw_start)).map_err(|e|e.to_string())?;
    let mut bytes=vec![0;(raw_end-raw_start) as usize];
    file.read_exact(&mut bytes).map_err(|_|"source_changed: history source was truncated")?;
    let at_path=std::fs::metadata(&source.segment.path).map_err(|_|"source_changed: history source was removed")?;
    if source::identity(&at_path)!=expected_identity || source::identity(&file.metadata().map_err(|e|e.to_string())?)!=expected_identity {
        return Err("source_changed: history source was replaced during the read".into());
    }
    Ok(bytes)
}

pub(crate) fn read_piece(conn:&Connection,source:&RegisteredSource,reference:&str,sequence:u64,depth:usize) -> Result<(String,u64,u64,u64),String> {
    if depth>2 {return Err("unsupported_record: nested detail encoding is too deep".into());}
    let (start,path,size,parent,encoding,field_start,field_end,field_hash):(u64,String,u64,Option<String>,String,u64,u64,Option<Vec<u8>>)=conn.query_row(
        "SELECT record_start,path,decoded_bytes,parent_ref,encoding,raw_start,raw_end,hash FROM fields WHERE ref=?1 AND source_id=?2 AND generation=?3",
        params![reference,source.id,source.generation],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?))).map_err(sql)?;
    if size==0 && sequence==0 {return Ok((String::new(),0,0,0));}
    if encoding=="json" || encoding=="slice" {
        const RAW_CHUNK:u64=8*1024;
        let relative=sequence.checked_mul(RAW_CHUNK).ok_or("cursor_expired: invalid JSON detail cursor")?;
        if relative>=size {return Err("cursor_expired: JSON detail cursor is outside its field".into());}
        let from=field_start+relative;
        let to=(from+RAW_CHUNK+4).min(field_end);
        let raw=if let Some(parent)=parent {
            let mut reader=DecodedFieldReader::new(conn,source,&parent,from,Some(to),depth+1)?;
            let mut raw=vec![0;(to-from) as usize]; reader.read_exact(&mut raw).map_err(|e|e.to_string())?; raw
        } else if let Some(expected)=field_hash.filter(|_|relative==0 && to==field_end && source.database.is_none() && source.kiro.is_none()) {
            // A scalar carries the hash of exactly its own bytes.
            let raw=original_range(conn,source,field_start,field_end)?;
            if Sha256::digest(&raw).as_slice()!=expected.as_slice() {changed(conn,source)?;}raw
        } else {verified_range(conn,source,from,to)?};
        let mut left=0;
        while left<raw.len() && raw[left]&0xc0==0x80 {left+=1;}
        let mut right=(RAW_CHUNK as usize).min(raw.len());
        while right<raw.len() && raw[right]&0xc0==0x80 {right+=1;}
        let text=std::str::from_utf8(&raw[left..right]).map_err(|_|"source_changed: JSON detail has invalid UTF-8")?.to_owned();
        return Ok((text,relative+left as u64,relative+right as u64,size));
    }
    let checkpoint:Option<(u64,u64,u64,u64,Vec<u8>)>=conn.query_row("SELECT raw_start,raw_end,decoded_start,decoded_end,hash FROM checkpoints
        WHERE source_id=?1 AND generation=?2 AND record_start=?3 AND path=?4 AND sequence=?5",
        params![source.id,source.generation,start,path,sequence],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional().map_err(sql)?;
    let Some((raw_start,raw_end,decoded_start,decoded_end,hash))=checkpoint else {
        // A string that fits in one piece has no checkpoint; its own hash verifies its raw range.
        let Some(expected)=field_hash.filter(|_|sequence==0 && encoding=="string" && field_end>=field_start
            && field_end-field_start<=(CHECKPOINT_BYTES+16) as u64) else {return Err("cursor_expired: detail checkpoint is unavailable".into());};
        let raw=if let Some(parent)=parent {
            let mut reader=DecodedFieldReader::new(conn,source,&parent,field_start,Some(field_end),depth+1)?;
            let mut bytes=vec![0;(field_end-field_start) as usize];reader.read_exact(&mut bytes).map_err(|e|e.to_string())?;bytes
        } else if source.database.is_some() || source.kiro.is_some() {verified_range(conn,source,field_start,field_end)?}
        else {original_range(conn,source,field_start,field_end)?};
        if Sha256::digest(&raw).as_slice()!=expected.as_slice() {changed(conn,source)?;}
        let text=decode_checkpoint(&raw)?;
        if text.len() as u64!=size {return Err("source_changed: detail size changed".into());}
        return Ok((text,0,size,size));
    };
    if raw_start<field_start || raw_end>field_end || decoded_start>=size || decoded_end>size {
        return Err("cursor_expired: detail checkpoint is outside its current field".into());
    }
    if raw_end<raw_start || raw_end-raw_start>(CHECKPOINT_BYTES+16) as u64 {return Err("History checkpoint exceeds its byte budget".into());}
    let raw=if let Some(parent)=parent {
        let mut reader=DecodedFieldReader::new(conn,source,&parent,raw_start,Some(raw_end),depth+1)?;
        let mut bytes=vec![0;(raw_end-raw_start) as usize];
        reader.read_exact(&mut bytes).map_err(|e|e.to_string())?;
        bytes
    } else if source.database.is_some() || source.kiro.is_some() {verified_range(conn,source,raw_start,raw_end)?} else {
        original_range(conn,source,raw_start,raw_end)?
    };
    if Sha256::digest(&raw).as_slice()!=hash {
        changed(conn,source)?;
    }
    Ok((decode_checkpoint(&raw)?,decoded_start,decoded_end,size))
}

/// Read an encoded inner JSON value directly through its outer string's verified decode checkpoints.
pub(crate) struct DecodedFieldReader<'a> {
    conn:&'a Connection,
    source:&'a RegisteredSource,
    reference:String,
    sequence:u64,
    start:u64,
    end:Option<u64>,
    depth:usize,
    current:std::io::Cursor<Vec<u8>>,
    done:bool,
}

impl<'a> DecodedFieldReader<'a> {
    pub(crate) fn new(conn:&'a Connection,source:&'a RegisteredSource,reference:&str,start:u64,end:Option<u64>,depth:usize) -> Result<Self,String> {
        let sequence:u64=conn.query_row("SELECT c.sequence FROM checkpoints c JOIN fields f ON f.source_id=c.source_id AND f.generation=c.generation
            AND f.record_start=c.record_start AND f.path=c.path WHERE f.ref=?1 AND c.decoded_start<=?2 ORDER BY c.decoded_start DESC LIMIT 1",
            params![reference,start],|r|r.get(0)).optional().map_err(sql)?.unwrap_or(0);
        Ok(Self {conn,source,reference:reference.into(),sequence,start,end,depth,current:std::io::Cursor::new(Vec::new()),done:false})
    }
}

impl Read for DecodedFieldReader<'_> {
    fn read(&mut self,output:&mut [u8]) -> std::io::Result<usize> {
        if output.is_empty() {return Ok(0);}
        loop {
            let count=self.current.read(output)?;
            if count>0 || self.done {return Ok(count);}
            let (text,start,end,size)=read_piece(self.conn,self.source,&self.reference,self.sequence,self.depth).map_err(std::io::Error::other)?;
            self.sequence+=1;
            let from=self.start.saturating_sub(start) as usize;
            let to=self.end.unwrap_or(end).min(end).saturating_sub(start) as usize;
            if from>to || to>text.len() {return Err(std::io::Error::other("cursor_expired: invalid decoded range"));}
            self.done=end>=size || self.end.is_some_and(|limit|end>=limit);
            self.current=std::io::Cursor::new(text.as_bytes()[from..to].to_vec());
        }
    }
}

pub fn prepare(app: &AppCtx, session_id: &str) -> Result<Branch, String> {
    let session = repo::get_session(&app.db().conn.lock().unwrap(),session_id).map_err(|e|e.to_string())?
        .ok_or("Session not found")?;
    let id = session.agent_session_id.unwrap_or_default();
    let store = Store::at(&app.data_dir()?)?;
    let mut sources = Vec::new();
    let protocols=super::protocol_source::published(&store,session.kind,session_id,&id)?;
    let resolved=if id.is_empty() {source::Resolved {segments:Vec::new(),error:None}} else {match source::resolve(session.kind,&id) {
        Ok(resolved)=>resolved,Err(error) if !protocols.is_empty()=>source::Resolved {segments:Vec::new(),error:Some(error)},Err(error)=>return Err(error),
    }};
    for segment in resolved.segments {
        sources.push(if session.kind==SessionKind::Kiro && segment.path.extension().and_then(|extension|extension.to_str())==Some("jsonl") {
                super::kiro_pair::register(&store,segment.path,&segment.native_id)?
            } else if session.kind==SessionKind::Kiro {super::kiro_database::register(&store,segment.path,&segment.native_id)?}
            else if session.kind==SessionKind::Opencode {super::native_database::register(&store,segment.path,&segment.native_id)?}
            else {store.register(session.kind,segment)?});
    }
    sources.extend(protocols);
    bind(store,sources,session.kind,id,resolved.error,session_id)
}

pub(crate) fn bind(store:Arc<Store>,sources:Vec<RegisteredSource>,kind:SessionKind,id:String,error:Option<String>,scope:&str) -> Result<Branch,String> {
    let conn=store.connect()?;
    let mut signature=Vec::new();
    for source in &sources {
        let revision:String=conn.query_row("SELECT branch_revision FROM sources WHERE id=?1",[&source.id],|r|r.get(0)).map_err(sql)?;
        signature.push(json!([source.id,source.generation,revision,source.segment.end_byte,source.segment.end_ordinal]));
    }
    let generation = digest(serde_json::to_string(&(signature,&error)).unwrap().as_bytes());
    conn.execute_batch("BEGIN IMMEDIATE").map_err(sql)?;
    conn.execute("INSERT INTO branches(session_scope,revision,error) VALUES(?1,?2,?3)
        ON CONFLICT(session_scope) DO UPDATE SET revision=excluded.revision,error=excluded.error",
        params![scope,generation,error]).map_err(sql)?;
    conn.execute("DELETE FROM segments WHERE session_scope=?1",[scope]).map_err(sql)?;
    for (index,source) in sources.iter().enumerate() {
        conn.execute("INSERT INTO segments VALUES(?1,?2,?3,?4,?5,?6)",params![scope,index as u64,source.id,source.generation,
            source.segment.end_byte,source.segment.end_ordinal]).map_err(sql)?;
    }
    conn.execute_batch("COMMIT").map_err(sql)?;
    for source in &sources {
        if kind==SessionKind::Codex && source.indexed==0 {
            let has_tail:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM records WHERE source_id=?1 AND generation=?2)",
                params![source.id,source.generation],|r|r.get(0)).map_err(sql)?;
            if !has_tail && source.size>TAIL_BYTES {
                store.scan(kind,source,&std::sync::atomic::AtomicBool::new(false),true)?;
            }
        }
        store.schedule(kind,source.clone());
    }
    Ok(Branch {store,sources,generation,kind,native_id:id,error,scope:scope.into(),parent:None})
}

fn field_view(reference:&str,label:&str,path:&str,size:u64,encoding:&str,append:bool,revision:&str) -> Value {
    let mut field=json!({"detailRef":reference,"label":label,"path":path,"sizeBytes":size,
        "kind":if encoding=="json" {"json"} else if path.ends_with("diff") {"diff"} else {"text"},"append":append,"contentRevision":revision});
    if let Some(label)=label.strip_prefix("attachment:") {field["kind"]=json!("image");field["label"]=json!("attachment");field["mimeType"]=json!(label.rsplit_once(':').map(|(mime,_)|mime));}
    match label {
        "body_shell"=>{field["label"]=json!("body");field["prefix"]=json!("!");},
        "body_file"=>{field["label"]=json!("body");field["prefix"]=json!("[file: ");field["suffix"]=json!("]");},
        "body_image"=>{field["label"]=json!("body");field["literal"]=json!("[image]");},
        _=>{},
    }field
}

impl Branch {
    pub(super) fn resolve_row(&self,conn:&Connection,id:&str) -> Result<String,String> {
        for source in self.sources.iter().rev() {
            let alias:Option<String>=conn.query_row("SELECT a.row_id FROM protocol_aliases a JOIN protocol_sources p ON p.source_id=a.source_id
                JOIN rows r ON r.source_id=a.source_id AND r.generation=a.generation AND r.id=a.row_id
                JOIN records original ON original.source_id=r.source_id AND original.generation=r.generation AND original.start=r.record_start
                WHERE a.source_id=?1 AND a.generation=?2 AND p.session_scope=?3 AND a.view_id=?4 AND original.active=1
                AND (?5 IS NULL OR r.record_end<=?5) AND (?6 IS NULL OR r.ordinal IS NULL OR r.ordinal<?6)",
                params![source.id,source.generation,self.scope,id,source.segment.end_byte,source.segment.end_ordinal],|row|row.get(0)).optional().map_err(sql)?;
            if let Some(alias)=alias {return Ok(alias);}
        }
        // A superseded row is located through the row that stands for it in the view.
        for source in &self.sources {
            let replacement:Option<String>=conn.query_row("SELECT replacement FROM superseded_rows WHERE source_id=?1 AND generation=?2 AND row_id=?3",
                params![source.id,source.generation,id],|row|row.get(0)).optional().map_err(sql)?;
            if let Some(replacement)=replacement {return Ok(replacement);}
        }
        Ok(id.into())
    }

    pub fn detail_facts(&self,reference:&str,expected:Option<&str>) -> Result<(String,u64),String> {
        let (source,_,_,size,_)=self.authorize(reference)?;
        let revision=format!("{}:{reference}",source.generation);
        if expected.is_some_and(|value|value!=revision) {return Err("source_changed: detail revision changed".into());}
        Ok((revision,size))
    }

    pub fn detail_sequence_at(&self,reference:&str,offset:u64) -> Result<u64,String> {
        let (source,start,path,size,_)=self.authorize(reference)?;
        if offset>=size && size>0 {return Err("cursor_expired: detail range is outside its field".into());}
        let conn=self.store.connect()?;
        let encoding:String=conn.query_row("SELECT encoding FROM fields WHERE ref=?1",[reference],|r|r.get(0)).map_err(sql)?;
        if encoding=="json" || encoding=="slice" {return Ok(offset.saturating_sub(3)/(8*1024));}
        conn.query_row("SELECT sequence FROM checkpoints WHERE source_id=?1 AND generation=?2 AND record_start=?3 AND path=?4
            AND decoded_start<=?5 ORDER BY decoded_start DESC LIMIT 1",params![source.id,source.generation,start,path,offset],|r|r.get(0))
            .optional().map_err(sql).map(|sequence|sequence.unwrap_or(0))
    }

    pub fn write_detail(&self,reference:&str,expected:Option<&str>,writer:&mut impl std::io::Write,cancel:&std::sync::atomic::AtomicBool) -> Result<u64,String> {
        let (revision,size)=self.detail_facts(reference,expected)?;
        let mut written=0;let mut sequence=0;
        while written<size {
            if cancel.load(std::sync::atomic::Ordering::Relaxed) {return Err("read_canceled".into());}
            let value=self.detail(reference,Some(&revision),sequence)?;
            let text=value["text"].as_str().ok_or("detail_unavailable: detail chunk is unavailable")?;
            writer.write_all(text.as_bytes()).map_err(|_|"storage_pressure: detail file could not be written")?;
            written+=text.len() as u64;sequence+=1;
            if value["eof"]==true {break;}
        }
        if written!=size {return Err("source_changed: detail size changed during the transfer".into());}
        Ok(written)
    }

    pub(super) fn cursor(&self,operation:&str,owner:&str,sequence:u64) -> String {
        let bound=serde_json::to_vec(&json!([2,self.scope,self.generation,operation,owner,sequence])).unwrap();
        let mut hash=Sha256::new(); hash.update(self.store.cursor_key.as_bytes()); hash.update(bound);
        let signature=hash.finalize().iter().map(|b|format!("{b:02x}")).collect::<String>();
        format!("v2.{sequence}.{signature}")
    }

    pub(super) fn cursor_sequence(&self,operation:&str,owner:&str,cursor:Option<&str>) -> Result<u64,String> {
        let Some(cursor)=cursor else {return Ok(0);};
        if cursor.len()>96 {return Err("cursor_expired: invalid history cursor".into());}
        let mut parts=cursor.split('.');
        if parts.next()!=Some("v2") {return Err("cursor_expired: unsupported history cursor".into());}
        let sequence=parts.next().and_then(|p|p.parse::<u64>().ok()).ok_or("cursor_expired: invalid history cursor")?;
        if self.cursor(operation,owner,sequence)!=cursor {return Err("cursor_expired: history cursor belongs to another scope or revision".into());}
        Ok(sequence)
    }

    pub fn detail_page(&self,reference:&str,expected:Option<&str>,cursor:Option<&str>) -> Result<Value,String> {
        let sequence=self.cursor_sequence("detail",reference,cursor)?;
        let mut value=self.detail(reference,expected,sequence)?;
        value["nextCursor"]=if value["eof"]==true {Value::Null} else {json!(self.cursor("detail",reference,sequence+1))};
        value["previousCursor"]=if sequence==0 {Value::Null} else {json!(self.cursor("detail",reference,sequence-1))};
        if serde_json::to_vec(&value).map_err(|e|e.to_string())?.len()>DETAIL_BYTES {return Err("History detail exceeds its wire byte budget".into());}
        Ok(value)
    }

    pub fn manifest_page(&self,row_id:&str,cursor:Option<&str>) -> Result<Value,String> {
        let from=self.cursor_sequence("manifest",row_id,cursor)?;
        let mut value=self.manifest(row_id,from)?;
        value["nextCursor"]=value["nextCursor"].as_u64().map(|next|json!(self.cursor("manifest",row_id,next))).unwrap_or(Value::Null);
        value["previousCursor"]=if from==0 {Value::Null} else {json!(self.cursor("manifest",row_id,from.saturating_sub(50)))};
        Ok(value)
    }

    /// A copied locator can mint fresh revision-bound cursors after a process or transport reconnect.
    /// The client supplies a logical chunk number, never a source path or byte offset.
    pub fn locate_detail(&self,row_id:&str,reference:&str,sequence:u64) -> Result<Value,String> {
        let (source,_,_,_,_)=self.authorize(reference)?;
        let conn=self.store.connect()?;
        let row_id=self.resolve_row(&conn,row_id)?;
        let bound:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM row_fields WHERE source_id=?1 AND generation=?2 AND row_id=?3 AND ref=?4)",
            params![source.id,source.generation,row_id,reference],|r|r.get(0)).map_err(sql)?;
        if !bound {return Err("detail_unavailable: field belongs to another history row".into());}
        let (path,size,encoding,label):(String,u64,String,String)=conn.query_row("SELECT f.path,f.decoded_bytes,f.encoding,rf.label FROM fields f
            JOIN row_fields rf ON rf.ref=f.ref WHERE f.ref=?1 AND rf.row_id=?2",params![reference,row_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).map_err(sql)?;
        let revision=format!("{}:{reference}",source.generation);
        Ok(json!({"field":field_view(reference,&label,&path,size,&encoding,false,&revision),
            "cursor":self.cursor("detail",reference,sequence),"historyGeneration":self.generation}))
    }

    pub(super) fn visible_sql(&self) -> String {
        let scope=self.scope.replace('\'',"''");
        let child=if self.kind==SessionKind::Codex && self.scope.starts_with("child:") {
            "AND EXISTS(SELECT 1 FROM records cr WHERE cr.source_id=r.source_id AND cr.generation=r.generation AND cr.start=r.record_start
                AND json_extract(cr.projection,'$.payload.type') IN ('item_completed','itemCompleted'))"
        } else {""};
        format!("r.source_id=?1 AND r.generation=?2 AND (?3 IS NULL OR r.record_end<=?3)
            AND (?4 IS NULL OR r.ordinal IS NULL OR r.ordinal<?4)
            AND EXISTS(SELECT 1 FROM records p WHERE p.source_id=r.source_id AND p.generation=r.generation AND p.start=r.record_start AND p.active=1) {child}
            AND NOT EXISTS(SELECT 1 FROM superseded_rows x WHERE x.source_id=r.source_id AND x.generation=r.generation AND x.row_id=r.id)
            AND NOT EXISTS(SELECT 1 FROM protocol_sources own JOIN rows native ON native.id=r.id
                JOIN segments ns ON ns.source_id=native.source_id AND ns.generation=native.generation
                JOIN sources native_source ON native_source.id=native.source_id AND native_source.generation=native.generation
                WHERE own.source_id=r.source_id AND own.session_scope='{scope}' AND ns.session_scope='{scope}'
                AND NOT EXISTS(SELECT 1 FROM protocol_sources WHERE source_id=native.source_id)
                AND (ns.end_byte IS NULL OR native.record_end<=ns.end_byte) AND (ns.end_ordinal IS NULL OR native.ordinal IS NULL OR native.ordinal<ns.end_ordinal)
                AND (native_source.provider NOT IN ('claude','pi','omp') OR native_source.status='complete')
                AND EXISTS(SELECT 1 FROM records nr WHERE nr.source_id=native.source_id AND nr.generation=native.generation AND nr.start=native.record_start AND nr.active=1))
            AND NOT EXISTS(SELECT 1 FROM protocol_sources own JOIN rows newer ON newer.id=r.id
                JOIN protocol_sources ps ON ps.source_id=newer.source_id
                JOIN segments ss ON ss.source_id=newer.source_id AND ss.generation=newer.generation
                WHERE own.source_id=r.source_id AND ss.session_scope='{scope}' AND ps.session_scope='{scope}' AND newer.source_id!=r.source_id
                AND (ss.end_byte IS NULL OR newer.record_end<=ss.end_byte) AND (ss.end_ordinal IS NULL OR newer.ordinal IS NULL OR newer.ordinal<ss.end_ordinal)
                AND EXISTS(SELECT 1 FROM records nr WHERE nr.source_id=newer.source_id AND nr.generation=newer.generation AND nr.start=newer.record_start AND nr.active=1)
                AND (ps.process_epoch>own.process_epoch OR ps.process_epoch=own.process_epoch AND (
                    (SELECT value FROM protocol_state WHERE source_id=ps.source_id AND generation=newer.generation AND key='direction')='incoming'
                        AND (SELECT value FROM protocol_state WHERE source_id=own.source_id AND generation=r.generation AND key='direction')='outgoing'
                    OR (SELECT value FROM protocol_state WHERE source_id=ps.source_id AND generation=newer.generation AND key='direction')=
                        (SELECT value FROM protocol_state WHERE source_id=own.source_id AND generation=r.generation AND key='direction') AND ps.rowid>own.rowid)))")
    }

    fn protocol_overlay<'a>(&'a self,conn:&Connection,id:&str) -> Result<Option<&'a RegisteredSource>,String> {
        let selected:Option<(String,String)>=conn.query_row("SELECT r.source_id,r.generation FROM rows r
            JOIN protocol_sources p ON p.source_id=r.source_id JOIN segments s ON s.source_id=r.source_id AND s.generation=r.generation
            JOIN records original ON original.source_id=r.source_id AND original.generation=r.generation AND original.start=r.record_start
            WHERE p.session_scope=?1 AND s.session_scope=?1 AND r.id=?2 AND original.active=1
            AND (s.end_byte IS NULL OR r.record_end<=s.end_byte) AND (s.end_ordinal IS NULL OR r.ordinal IS NULL OR r.ordinal<s.end_ordinal)
            ORDER BY p.process_epoch DESC,(SELECT value='incoming' FROM protocol_state WHERE source_id=p.source_id AND generation=r.generation AND key='direction') DESC,
                p.rowid DESC LIMIT 1",params![self.scope,id],|row|Ok((row.get(0)?,row.get(1)?))).optional().map_err(sql)?;
        Ok(selected.and_then(|(id,generation)|self.sources.iter().find(|source|source.id==id && source.generation==generation)))
    }

    pub(super) fn render_row(&self,conn:&Connection,source:&RegisteredSource,row:&Value) -> Result<Value,String> {
        if let Some(overlay)=self.protocol_overlay(conn,row["id"].as_str().unwrap_or_default())? {
            if overlay.id!=source.id {
                let raw:String=conn.query_row("SELECT summary FROM rows WHERE source_id=?1 AND generation=?2 AND id=?3",params![overlay.id,overlay.generation,row["id"].as_str()],|row|row.get(0)).map_err(sql)?;
                return self.render_row(conn,overlay,&serde_json::from_str(&raw).map_err(|e|e.to_string())?);
            }
        }
        let protocol:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM protocol_sources WHERE source_id=?1)",[&source.id],|row|row.get(0)).map_err(sql)?;
        if self.kind==SessionKind::Opencode || protocol && (self.kind==SessionKind::Codex || row["kind"]!="tool") {return Ok(super::children::public_row(super::attachments::decorate(conn,source,row.clone())?));}
        let mut stmt=conn.prepare("SELECT p.summary FROM row_parts p JOIN records r ON r.source_id=p.source_id AND r.generation=p.generation AND r.start=p.record_start
            WHERE p.source_id=?1 AND p.generation=?2 AND p.row_id=?3 AND r.active=1
            AND (?4 IS NULL OR p.record_end<=?4) AND (?5 IS NULL OR p.ordinal IS NULL OR p.ordinal<?5) ORDER BY p.record_start").map_err(sql)?;
        let mut parts=stmt.query(params![source.id,source.generation,row["id"].as_str(),source.segment.end_byte,source.segment.end_ordinal]).map_err(sql)?;
        let mut result=None;
        while let Some(part)=parts.next().map_err(sql)? {
            let raw:String=part.get(0).map_err(sql)?;
            let mut value:Value=serde_json::from_str(&raw).map_err(|e|e.to_string())?;
            if let Some(old)=result.take() {
                let old:Value=old;
                super::children::merge(&old,&mut value);
                if value["kind"]=="tool" && value.get("output").is_some() {
                    value["name"]=old["name"].clone(); value["input"]=old["input"].clone();
                    if value.get("_child").is_none() {if let Some(child)=old.get("_child") {value["_child"]=child.clone();}}
                } else if value["kind"]=="tool" && old.get("output").is_some() {
                    value["output"]=old["output"].clone(); value["status"]=old["status"].clone(); value["isError"]=old["isError"].clone();
                }
            }
            result=Some(value);
        }
        Ok(super::children::public_row(super::attachments::decorate(conn,source,result.unwrap_or_else(||row.clone()))?))
    }

    pub fn forward(&self,after:Option<&str>) -> Result<Vec<(String,Value)>,String> {
        let conn=self.store.connect()?;
        let mut result=Vec::new();
        for (sequence,source) in self.sources.iter().enumerate() {
            let mut stmt=conn.prepare(&format!("SELECT r.order_key,r.summary FROM rows r WHERE {} AND (?5 IS NULL OR printf('%04d:',?6)||r.order_key>?5)
                ORDER BY r.order_key LIMIT ?7",self.visible_sql())).map_err(sql)?;
            let values=stmt.query_map(params![source.id,source.generation,source.segment.end_byte,source.segment.end_ordinal,after,sequence as u64,PAGE_ROWS],
                |r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?))).map_err(sql)?;
            for value in values {
                let (order,summary)=value.map_err(sql)?;
                let row:Value=serde_json::from_str(&summary).map_err(|e|e.to_string())?;
                result.push((format!("{sequence:04}:{order}"),self.render_row(&conn,source,&row)?));
                if result.len()>=PAGE_ROWS {return Ok(result);}
            }
        }
        Ok(result)
    }

    pub fn authorize(&self, reference: &str) -> Result<(RegisteredSource,u64,String,u64,Option<String>),String> {
        super::children::validate_parent(self)?;
        let conn=self.store.connect()?;
        let field: Option<(String,String,u64,String,u64,Option<String>)>=conn.query_row(
            "SELECT source_id,generation,record_start,path,decoded_bytes,parent_ref FROM fields WHERE ref=?1",[reference],
            |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).optional().map_err(sql)?;
        let (id,generation,start,path,size,parent)=field.ok_or("detail_unavailable: history detail is unavailable")?;
        let source=self.sources.iter().find(|s|s.id==id && s.generation==generation)
            .ok_or("source_changed: detail belongs to an expired history generation")?.clone();
        let allowed: bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM records WHERE source_id=?1 AND generation=?2 AND start=?3 AND active=1
            AND (?4 IS NULL OR end<=?4) AND (?5 IS NULL OR ordinal IS NULL OR ordinal<?5)
            AND EXISTS(SELECT 1 FROM row_fields rf CROSS JOIN rows rw ON rw.source_id=rf.source_id AND rw.generation=rf.generation AND rw.id=rf.row_id
                JOIN records rr ON rr.source_id=rw.source_id AND rr.generation=rw.generation AND rr.start=rw.record_start
                WHERE rf.ref=?6 AND rf.source_id=?1 AND rf.generation=?2 AND rr.active=1
                AND (?4 IS NULL OR rw.record_end<=?4) AND (?5 IS NULL OR rw.ordinal IS NULL OR rw.ordinal<?5)))",
            params![id,generation,start,source.segment.end_byte,source.segment.end_ordinal,reference],|r|r.get(0)).map_err(sql)?;
        if !allowed {return Err("detail_unavailable: detail is outside the selected history branch".into());}
        Ok((source,start,path,size,parent))
    }

    pub fn detail(&self, reference:&str, expected:Option<&str>, sequence:u64) -> Result<Value,String> {
        let (source,_,_,_,_)=self.authorize(reference)?;
        let revision=format!("{}:{reference}",source.generation);
        if expected.is_some_and(|value|value!=revision) {return Err("source_changed: detail revision changed".into());}
        let conn=self.store.connect()?;
        let (text,decoded_start,decoded_end,size)=read_piece(&conn,&source,reference,sequence,0)?;
        let eof=decoded_end>=size;
        let result=json!({"contentRevision":revision,"chunkSequence":sequence,"text":text,"nextCursor":if eof {None} else {Some(sequence+1)},
            "eof":eof,"decodedStart":decoded_start,"decodedEnd":decoded_end});
        if serde_json::to_vec(&result).map_err(|e|e.to_string())?.len()>DETAIL_BYTES {return Err("History detail exceeds its wire byte budget".into());}
        Ok(result)
    }

    /// The same branch ending where the running process began: native sources stop at the sizes
    /// captured before launch and that process's own protocol sources are left out. The generation
    /// stays the full branch's, so details read from these rows authorize against the full branch.
    pub fn before_process(&self,cut:&ProcessCut) -> Result<Branch,String> {
        let conn=self.store.connect()?;
        let mut sources=Vec::with_capacity(self.sources.len());
        for source in &self.sources {
            let epoch:Option<u64>=conn.query_row("SELECT process_epoch FROM protocol_sources WHERE source_id=?1",[&source.id],|r|r.get(0)).optional().map_err(sql)?;
            if epoch==Some(cut.epoch) {continue;}
            let mut source=source.clone();
            if epoch.is_none() {
                // A recording that did not exist at launch was written entirely by this process.
                let size=cut.sizes.iter().find(|(path,_)|*path==source.segment.path).map_or(0,|(_,size)|*size);
                source.segment.end_byte=Some(source.segment.end_byte.map_or(size,|end|end.min(size)));
            }
            sources.push(source);
        }
        Ok(Branch {sources,..self.clone()})
    }

    pub fn page(&self, before:Option<&str>, after:Option<&str>, anchor:Option<&str>) -> Result<Value,String> {
        self.page_with(before,after,anchor,false)
    }

    /// `live` marks a branch from [`Branch::before_process`]: the running agent keeps appending past
    /// the cut, and a tree provider's earlier rows stay addressable once the scan has passed the cut.
    pub fn page_with(&self, before:Option<&str>, after:Option<&str>, anchor:Option<&str>, live:bool) -> Result<Value,String> {
        super::children::validate_parent(self)?;
        let conn=self.store.connect()?;
        let mut candidates: Vec<(String,Value,u64)>=Vec::new();
        let mut indexed=0u64;
        let mut bytes=0u64;
        let mut known=0u64;
        let mut complete=self.error.is_none();
        let mut indexing=false;
        let mut error=self.error.clone();
        let boundary=before.or(after).or(anchor);
        let mut located=None;
        let mut canonical_anchor=None;
        if let Some(id)=boundary {
            let id=self.resolve_row(&conn,id)?;
            if anchor.is_some() {canonical_anchor=Some(id.clone());}
            for (sequence,source) in self.sources.iter().enumerate() {
                let order:Option<String>=conn.query_row(&format!("SELECT r.order_key FROM rows r WHERE {} AND r.id=?5",self.visible_sql()),
                    params![source.id,source.generation,source.segment.end_byte,source.segment.end_ordinal,id],|r|r.get(0)).optional().map_err(sql)?;
                if let Some(order)=order {located=Some(format!("{sequence:04}:{order}"));break;}
            }
            if located.is_none() {
                let mut pending=self.page_with(None,None,None,live)?;
                if pending["indexing"]==true {
                    let mut location=serde_json::Map::new();
                    for (key,value) in [("before",before),("after",after),("anchor",anchor)] {
                        if let Some(value)=value {location.insert(key.into(),json!(value));}
                    }
                    pending["pendingLocation"]=Value::Object(location);
                    pending["rows"]=json!([]);pending["positions"]=json!({});pending["hasMore"]=json!(false);pending["hasNewer"]=json!(false);
                    return Ok(pending);
                }
                return Err("cursor_expired: history row is outside the current branch".into());
            }
        }
        for (sequence,source) in self.sources.iter().enumerate() {
            let (water,status,source_error):(u64,String,Option<String>)=conn.query_row("SELECT indexed_bytes,status,error FROM sources WHERE id=?1 AND generation=?2",
                params![source.id,source.generation],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(sql)?;
            indexed+=water.min(source.segment.end_byte.unwrap_or(source.size));
            bytes+=source.segment.end_byte.unwrap_or(source.size);
            complete&=status=="complete" || source.segment.end_byte.is_some_and(|end|water>=end);
            indexing|=status=="partial" && (water<source.segment.end_byte.unwrap_or(source.size) || source.kiro_pair.is_some());
            error=error.or(source_error).or_else(||(status=="error").then(||"History contains unsupported or malformed records".into()));
            // Tree providers cannot claim an active branch before its leaf has been confirmed. While an
            // appended tail is read, the branch confirmed by the last complete scan stays visible.
            let mut end=source.segment.end_byte;
            if matches!(self.kind,SessionKind::Claude|SessionKind::Pi|SessionKind::Omp) && status!="complete"
                && !(live && source.segment.end_byte.is_some_and(|end|water>=end)) {
                let mark:Option<u64>=conn.query_row("SELECT selected_end FROM branch_marks WHERE source_id=?1 AND generation=?2",
                    params![source.id,source.generation],|r|r.get(0)).optional().map_err(sql)?;
                match mark.filter(|mark|*mark>0) {Some(mark)=>end=Some(end.map_or(mark,|end|end.min(mark))),None=>continue}
            }
            let count:u64=conn.query_row(&format!("SELECT count(*) FROM rows r WHERE {}",self.visible_sql()),
                params![source.id,source.generation,end,source.segment.end_ordinal],|r|r.get(0)).map_err(sql)?;
            let forward=after.is_some();
            let comparison=if forward {">"} else if anchor.is_some() {"<="} else {"<"};
            let query=format!("SELECT r.order_key,r.summary FROM rows r WHERE {} AND (?5 IS NULL OR printf('%04d:',?6)||r.order_key {comparison} ?5)
                ORDER BY r.order_key {} LIMIT ?7",self.visible_sql(),if forward {"ASC"} else {"DESC"});
            let mut stmt=conn.prepare(&query).map_err(sql)?;
            let selected=stmt.query_map(params![source.id,source.generation,end,source.segment.end_ordinal,located,sequence as u64,PAGE_ROWS+1],
                |r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?))).map_err(sql)?;
            for entry in selected {
                let (order,summary)=entry.map_err(sql)?;
                let value:Value=serde_json::from_str(&summary).map_err(|e|e.to_string())?;
                let value=self.render_row(&conn,source,&value)?;
                let position:u64=conn.query_row(&format!("SELECT count(*) FROM rows r WHERE {} AND r.order_key<?5",self.visible_sql()),
                    params![source.id,source.generation,end,source.segment.end_ordinal,order],|r|r.get(0)).map_err(sql)?;
                candidates.push((format!("{sequence:04}:{order}"),value,known+position));
            }
            known+=count;
        }
        candidates.sort_by(|a,b|a.0.cmp(&b.0));
        if after.is_some() {candidates.truncate(PAGE_ROWS);}
        else if candidates.len()>PAGE_ROWS {candidates.drain(..candidates.len()-PAGE_ROWS);}
        let mut rows=Vec::new();
        let mut positions=HashMap::new();
        for (_,row,position) in candidates {
            positions.insert(row["id"].as_str().unwrap_or_default().to_owned(),position);
            rows.push(row);
        }
        let coverage=if error.is_some() {"error"} else if complete {"complete"} else {"partial"};
        let mut result=json!({"protocolVersion":2,"historyGeneration":self.generation,"processEpoch":null,
            "snapshotRevision":indexed.to_string(),"coverage":coverage,"indexing":indexing,"indexedBytes":indexed,"sourceBytes":bytes,"knownRows":known,
            "totalRows":if complete {Some(known)} else {None},"rowsRevision":indexed,"positions":positions,"rows":rows,
            "pageKind":if before.is_some() {"history"} else {"recent"},"hasMore":false,"historyError":error,"agentSessionId":self.native_id});
        if let Some(id)=canonical_anchor {result["locatedRowId"]=json!(id);}
        while serde_json::to_vec(&result).map_err(|e|e.to_string())?.len()>PAGE_BYTES-16*1024 {
            let rows=result["rows"].as_array_mut().unwrap();
            if rows.len()<=1 {return Err("History row exceeds its page byte budget".into());}
            let removed=rows.remove(if after.is_some() {rows.len()-1} else {0});
            result["positions"].as_object_mut().unwrap().remove(removed["id"].as_str().unwrap());
        }
        let first=result["rows"].as_array().and_then(|r|r.first()).and_then(|r|r["id"].as_str());
        let last=result["rows"].as_array().and_then(|r|r.last()).and_then(|r|r["id"].as_str());
        let has_more=first.and_then(|id|result["positions"][id].as_u64()).is_some_and(|position|position>0);
        let has_newer=last.and_then(|id|result["positions"][id].as_u64()).is_some_and(|position|position+1<known);
        result["hasMore"]=json!(has_more);
        result["hasNewer"]=json!(has_newer);
        Ok(result)
    }

    pub fn manifest(&self,row_id:&str,from:u64) -> Result<Value,String> {
        super::children::validate_parent(self)?;
        let conn=self.store.connect()?;
        let row_id=self.resolve_row(&conn,row_id)?;
        let overlay=self.protocol_overlay(&conn,&row_id)?;
        let mut sources=Vec::with_capacity(self.sources.len()+1);
        if let Some(source)=overlay {sources.push(source);}
        sources.extend(self.sources.iter().filter(|source|overlay.is_none_or(|overlay|source.id!=overlay.id)));
        for source in sources {
            let visibility=if overlay.is_some_and(|overlay|overlay.id==source.id) {
                "r.source_id=?1 AND r.generation=?2 AND (?3 IS NULL OR r.record_end<=?3) AND (?4 IS NULL OR r.ordinal IS NULL OR r.ordinal<?4)".into()
            } else {self.visible_sql()};
            let summary:Option<String>=conn.query_row(&format!("SELECT r.summary FROM rows r WHERE {visibility} AND r.id=?5"),
                params![source.id,source.generation,source.segment.end_byte,source.segment.end_ordinal,row_id],|r|r.get(0)).optional().map_err(sql)?;
            let Some(summary)=summary else {continue;};
            let row:Value=serde_json::from_str(&summary).map_err(|e|e.to_string())?;
            let row=self.render_row(&conn,source,&row)?;
            let mut fields=Vec::new();
            let mut stmt=conn.prepare("SELECT f.ref,rf.label,f.path,f.decoded_bytes,f.encoding,EXISTS(SELECT 1 FROM append_fields WHERE ref=f.ref) FROM row_fields rf JOIN fields f ON f.ref=rf.ref
                JOIN records r ON r.source_id=f.source_id AND r.generation=f.generation AND r.start=f.record_start
                WHERE rf.source_id=?1 AND rf.generation=?2 AND rf.row_id=?3 AND r.active=1
                AND (?5 IS NULL OR r.end<=?5) AND (?6 IS NULL OR r.ordinal IS NULL OR r.ordinal<?6)
                AND (EXISTS(SELECT 1 FROM append_fields WHERE ref=f.ref) OR NOT EXISTS(SELECT 1 FROM row_fields newer JOIN fields nf ON nf.ref=newer.ref JOIN records nr
                    ON nr.source_id=nf.source_id AND nr.generation=nf.generation AND nr.start=nf.record_start
                    WHERE newer.source_id=rf.source_id AND newer.generation=rf.generation AND newer.row_id=rf.row_id
                    AND newer.label=rf.label AND nf.path=f.path AND nf.record_start>f.record_start AND nr.active=1
                    AND (?5 IS NULL OR nr.end<=?5) AND (?6 IS NULL OR nr.ordinal IS NULL OR nr.ordinal<?6)))
                ORDER BY rf.sequence LIMIT 51 OFFSET ?4").map_err(sql)?;
            let values=stmt.query_map(params![source.id,source.generation,row_id,from,source.segment.end_byte,source.segment.end_ordinal],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,u64>(3)?,r.get::<_,String>(4)?,r.get::<_,bool>(5)?))).map_err(sql)?;
            for value in values {
                let (reference,label,path,size,encoding,append)=value.map_err(sql)?;
                fields.push(field_view(&reference,&label,&path,size,&encoding,append && source.database.is_none(),&format!("{}:{reference}",source.generation)));
            }
            let more=fields.len()>50;
            fields.truncate(50);
            let mut files=Vec::new();
            let mut stmt=conn.prepare("SELECT f.path,f.change_type,f.detail_ref,f.detail_kind FROM files f JOIN records r
                ON r.source_id=f.source_id AND r.generation=f.generation AND r.start=f.record_start
                WHERE f.source_id=?1 AND f.generation=?2 AND f.row_id=?3 AND r.active=1
                AND (?5 IS NULL OR r.end<=?5) AND (?6 IS NULL OR r.ordinal IS NULL OR r.ordinal<?6)
                AND f.record_start=(SELECT max(p.record_start) FROM files p JOIN records q
                    ON q.source_id=p.source_id AND q.generation=p.generation AND q.start=p.record_start
                    WHERE p.source_id=f.source_id AND p.generation=f.generation AND p.row_id=f.row_id AND q.active=1
                    AND (?5 IS NULL OR q.end<=?5) AND (?6 IS NULL OR q.ordinal IS NULL OR q.ordinal<?6))
                ORDER BY f.file_ordinal LIMIT 51 OFFSET ?4").map_err(sql)?;
            let values=stmt.query_map(params![source.id,source.generation,row_id,from,source.segment.end_byte,source.segment.end_ordinal],|r|Ok((r.get::<_,String>(0)?,r.get::<_,Option<String>>(1)?,r.get::<_,Option<String>>(2)?,r.get::<_,Option<String>>(3)?))).map_err(sql)?;
            for value in values {
                let (path,change,reference,kind)=value.map_err(sql)?;
                files.push(json!({"path":path,"changeType":change,"detailRef":reference,"kind":kind,
                    "contentRevision":reference.as_ref().map(|r|format!("{}:{r}",source.generation))}));
            }
            let more=more||files.len()>50;
            files.truncate(50);
            let value=json!({"protocolVersion":2,"historyGeneration":self.generation,"row":row,"fields":fields,"files":files,"nextCursor":if more {Some(from+50)} else {None}});
            if serde_json::to_vec(&value).map_err(|e|e.to_string())?.len()>PAGE_BYTES {return Err("History manifest exceeds its wire byte budget".into());}
            return Ok(value);
        }
        Err("detail_unavailable: history row is unavailable".into())
    }
}

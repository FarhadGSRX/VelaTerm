//! Bounded, read-only projection over native SQLite TEXT/BLOB columns in one transaction snapshot.

use std::io::{Read,Seek,SeekFrom};
use std::path::Path;

use rusqlite::{Connection,DatabaseName,OpenFlags,OptionalExtension};
use sha2::{Digest,Sha256};

use super::limits::CHECKPOINT_BYTES;
use super::projection::{Checkpoint,Projector,Record};

#[derive(Clone,Copy)]
pub enum Entry {OpenCodeMessage,OpenCodePart,KiroConversation}
impl Entry {
    pub(super) fn table(self) -> &'static str {match self {Self::OpenCodeMessage=>"message",Self::OpenCodePart=>"part",Self::KiroConversation=>"conversations_v2"}}
    pub(super) fn column(self) -> &'static str {match self {Self::KiroConversation=>"value",_=>"data"}}
    fn scope(self) -> &'static str {match self {Self::KiroConversation=>"conversation_id",_=>"session_id"}}
}

pub fn open(path:&Path) -> Result<Connection,String> {
    let conn=Connection::open_with_flags(path,OpenFlags::SQLITE_OPEN_READ_ONLY|OpenFlags::SQLITE_OPEN_NO_MUTEX)
        .map_err(|_|"detail_unavailable: native database is unavailable")?;
    conn.busy_timeout(std::time::Duration::from_secs(2)).map_err(|_|"Cannot configure the native database reader")?;
    conn.execute_batch("PRAGMA cache_size=-8192; PRAGMA temp_store=FILE; PRAGMA query_only=ON; BEGIN;").map_err(|_|"Cannot open a native history snapshot")?;
    Ok(conn)
}

fn authorize(conn:&Connection,entry:Entry,rowid:i64,session:&str) -> Result<(),String> {
    let present:bool=conn.query_row(&format!("SELECT EXISTS(SELECT 1 FROM {} WHERE rowid=?1 AND {}=?2)",entry.table(),entry.scope()),
        rusqlite::params![rowid,session],|r|r.get(0)).map_err(|_|"unsupported_record: native database schema is unavailable")?;
    if !present {return Err("detail_unavailable: database entry is outside the selected session".into());}
    Ok(())
}

struct Hashed<R:Read> {reader:R,hash:Sha256,bytes:u64}
impl<R:Read> Read for Hashed<R> {
    fn read(&mut self,output:&mut [u8]) -> std::io::Result<usize> {
        let count=self.reader.read(output)?;self.hash.update(&output[..count]);self.bytes+=count as u64;Ok(count)
    }
}

pub struct Projection {pub record:Record,pub hash:Vec<u8>,pub bytes:u64}

pub fn project(conn:&Connection,entry:Entry,rowid:i64,session:&str,sink:&mut impl FnMut(Checkpoint)->Result<(),String>) -> Result<Projection,String> {
    authorize(conn,entry,rowid,session)?;
    let blob=conn.blob_open(DatabaseName::Main,entry.table(),entry.column(),rowid,true)
        .map_err(|_|"unsupported_record: native history column cannot be read incrementally")?;
    let size=blob.len() as u64;
    let mut reader=Hashed {reader:blob,hash:Sha256::new(),bytes:0};
    let mut record=Projector::document(&mut reader,0).next(sink)?.ok_or("unsupported_record: native history entry is empty")?;
    if record.error.is_none() && record.end!=size {return Err("unsupported_record: native history entry contains trailing records".into());}
    // Database commits are durable boundaries; they do not require a JSONL newline byte.
    record.complete=true;
    Ok(Projection {record,hash:reader.hash.finalize().to_vec(),bytes:reader.bytes})
}

/// Return the same bounded bytes that were checked, while keeping ownership and the BLOB handle in the
/// caller's read transaction. No path re-open or unrelated-session lookup occurs between check and read.
pub fn range(conn:&Connection,entry:Entry,rowid:i64,session:&str,start:u64,end:u64,expected:&[u8]) -> Result<Vec<u8>,String> {
    if end<start || end-start>(CHECKPOINT_BYTES+16) as u64 {return Err("History database range exceeds its byte budget".into());}
    authorize(conn,entry,rowid,session)?;
    let mut blob=conn.blob_open(DatabaseName::Main,entry.table(),entry.column(),rowid,true)
        .map_err(|_|"source_changed: native database entry is unavailable")?;
    if end>blob.len() as u64 {return Err("source_changed: native database entry was truncated".into());}
    blob.seek(SeekFrom::Start(start)).map_err(|_|"cursor_expired: invalid database detail range")?;
    let mut bytes=vec![0;(end-start) as usize];blob.read_exact(&mut bytes).map_err(|_|"source_changed: native database entry changed")?;
    if Sha256::digest(&bytes).as_slice()!=expected {return Err("source_changed: native database detail revision changed".into());}
    Ok(bytes)
}

/// Message metadata and native identity are selected before touching any large part column.
pub fn last_open_code_usage(conn:&Connection,session:&str) -> Result<serde_json::Value,String> {
    let revert:Option<(Option<u64>,Option<String>)>=conn.query_row("SELECT octet_length(revert),
        CASE WHEN octet_length(revert)<=65536 THEN revert ELSE NULL END FROM session WHERE id=?1",
        [session],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(|_|"Cannot inspect OpenCode's session revision")?;
    let (bytes,raw)=revert.ok_or("OpenCode has no recorded session")?;
    if bytes.is_some_and(|bytes|bytes>65536) {return Err("unsupported_record: OpenCode revert metadata exceeds its byte budget".into());}
    let revert=raw.map(|raw|serde_json::from_str::<serde_json::Value>(&raw)
        .map_err(|_|"unsupported_record: OpenCode revert metadata is invalid")).transpose()?;
    let cut=revert.as_ref().and_then(crate::agent::opencode_store::reverted_from);
    let cutoff:Option<(i64,String)>=if let Some(id)=cut {conn.query_row("SELECT time_created,id FROM message WHERE session_id=?1 AND id=?2",
        rusqlite::params![session,id],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(|_|"Cannot inspect OpenCode's revert boundary")?} else {None};
    if cut.is_some() && cutoff.is_none() {return Err("index_incomplete: OpenCode revert boundary is unavailable".into());}
    let mut stmt=conn.prepare("SELECT rowid FROM message WHERE session_id=?1 AND (?2 IS NULL OR time_created<?2 OR (time_created=?2 AND id<?3))
        ORDER BY time_created DESC,id DESC").map_err(|_|"Cannot inspect OpenCode message metadata")?;
    let mut rows=stmt.query(rusqlite::params![session,cutoff.as_ref().map(|(time,_)|*time),cutoff.as_ref().map(|(_,id)|id)])
        .map_err(|_|"Cannot read OpenCode message metadata")?;
    while let Some(row)=rows.next().map_err(|_|"Cannot read OpenCode message metadata")? {
        let projection=project(conn,Entry::OpenCodeMessage,row.get(0).map_err(|_|"Invalid OpenCode message identity")?,session,&mut |_|Ok(()))?;
        if let Some(error)=projection.record.error {return Err(error);}
        let value=projection.record.value;
        if value["role"]=="assistant" && value.pointer("/tokens/total").and_then(serde_json::Value::as_u64).is_some_and(|total|total>0) {return Ok(value);}
    }
    Err("OpenCode session has no recorded usage yet".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use super::super::limits::INPUT_BYTES;

    #[test]
    fn sqlite_text_projection_is_bounded_and_reads_late_metadata() {
        let conn=Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE message(id TEXT,session_id TEXT,time_created INTEGER,data TEXT);CREATE TABLE session(id TEXT,revert TEXT);").unwrap();
        conn.execute("INSERT INTO session VALUES('selected',NULL)",[]).unwrap();
        let raw=format!("{{\"unknown\":\"{}\",\"role\":\"assistant\",\"modelID\":\"native-model\",\"tokens\":{{\"total\":12,\"input\":10}}}}","x".repeat(2*INPUT_BYTES));
        conn.execute("INSERT INTO message VALUES('native','selected',1,?1)",[&raw]).unwrap();
        let mut checkpoints=Vec::new();let projected=project(&conn,Entry::OpenCodeMessage,1,"selected",&mut |checkpoint|{checkpoints.push(checkpoint);Ok(())}).unwrap();
        assert_eq!(projected.record.error,None);assert!(projected.record.complete);
        assert_eq!(projected.record.value["role"],"assistant");assert!(projected.record.value["unknown"].as_str().unwrap().len()<raw.len());
        assert_eq!(projected.bytes,raw.len() as u64);assert_eq!(last_open_code_usage(&conn,"selected").unwrap()["modelID"],"native-model");
        let checkpoint=&checkpoints[0];
        let bytes=range(&conn,Entry::OpenCodeMessage,1,"selected",checkpoint.raw_start,checkpoint.raw_end,&checkpoint.hash).unwrap();
        assert_eq!(super::super::projection::decode_checkpoint(&bytes).unwrap().len(),CHECKPOINT_BYTES);
        assert!(project(&conn,Entry::OpenCodeMessage,1,"foreign",&mut |_|Ok(())).err().unwrap().contains("outside the selected session"));
        conn.execute("UPDATE message SET data=?1",[raw.replace('x',"y")]).unwrap();
        assert!(range(&conn,Entry::OpenCodeMessage,1,"selected",checkpoint.raw_start,checkpoint.raw_end,&checkpoint.hash).unwrap_err().starts_with("source_changed:"));
    }

    #[test]
    fn database_documents_allow_structural_newlines_and_respect_revert_order() {
        let conn=Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE message(id TEXT,session_id TEXT,time_created INTEGER,data TEXT);CREATE TABLE session(id TEXT,revert TEXT);").unwrap();
        conn.execute("INSERT INTO session VALUES('selected',?1)",["{\"messageID\":\"reverted\"}"]).unwrap();
        for (id,time,model) in [("visible",1,"first"),("reverted",2,"second")] {
            let raw=serde_json::to_string_pretty(&json!({"role":"assistant","modelID":model,"tokens":{"total":1}})).unwrap();
            conn.execute("INSERT INTO message VALUES(?1,'selected',?2,?3)",rusqlite::params![id,time,raw]).unwrap();
        }
        assert_eq!(last_open_code_usage(&conn,"selected").unwrap()["modelID"],"first");
    }
}

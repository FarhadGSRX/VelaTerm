//! Persistent global search index, kept in its own derived file outside the business database.
//!
//! One row is one locatable fragment: a block of at most [`CHUNK_BYTES`] of a transcript message, or one
//! visible recording line. Each fragment goes into the trigram table `session_fts` and, under the same
//! rowid, into the word table `session_words` with jieba boundaries marked; the query side keeps the
//! semantics of the earlier in-database index (words first, then trigram, then LIKE for short terms).
//!
//! Transcripts are read through the bounded prose reader one message at a time, never as a whole file.
//! A session is indexed in one transaction, so a search sees either its previous rows or its new rows.
//! Each session records a fingerprint of its source; an unchanged session costs one file stat, and an
//! append that keeps the history generation re-indexes only from the last indexed message on.
//!
//! The file can be deleted at any time: it is rebuilt from the original recordings.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path,PathBuf};
use std::rc::Rc;
use std::sync::{Arc,Mutex,OnceLock};
use std::sync::atomic::{AtomicBool,Ordering};

use rusqlite::{params,params_from_iter,Connection,OptionalExtension};

use crate::agent::history_store::{self as history,api::Branch,prose::{Prose,Start},prose_reader::Reader,store::Store};
use crate::agent::transcript;
use crate::host::AppCtx;
use crate::models::SessionKind;
use super::{tokenize,SearchMatch,SearchScope,SessionSearchHit,limit_chars};

/// Extraction and schema version; a different stored version rebuilds the whole file.
const VERSION:i64=2;
/// Bytes of message text per fragment. Phrase and AND matches hold within one fragment.
pub(super) const CHUNK_BYTES:usize=64*1024;
/// Snippets returned per session; further fragments only count toward `match_count`.
pub(super) const MAX_MATCHES:usize=50;
/// Minimum term length for trigram MATCH; shorter terms use LIKE.
const MIN_FTS_TERM_CHARS:usize=3;
/// Bytes of readable context kept on each side of a match in a snippet.
const SNIPPET_CONTEXT:usize=40;

const SCHEMA:&str="
CREATE TABLE IF NOT EXISTS corpus_meta(version INTEGER NOT NULL);
CREATE VIRTUAL TABLE IF NOT EXISTS session_fts USING fts5(
  text, session_id UNINDEXED, source UNINDEXED, message_index UNINDEXED, ordinal UNINDEXED, role UNINDEXED, ts UNINDEXED,
  tokenize = 'trigram remove_diacritics 1');
CREATE VIRTUAL TABLE IF NOT EXISTS session_words USING fts5(
  words, session_id UNINDEXED, tokenize = 'porter unicode61 remove_diacritics 1');
CREATE TABLE IF NOT EXISTS corpus_state(
  session_id TEXT PRIMARY KEY, source TEXT NOT NULL, path TEXT, length INTEGER NOT NULL, modified INTEGER,
  generation TEXT, messages INTEGER NOT NULL, indexed_at INTEGER NOT NULL, resume TEXT);
";

fn sql(error:rusqlite::Error) -> String {
    if matches!(error,rusqlite::Error::SqliteFailure(rusqlite::ffi::Error {code:rusqlite::ErrorCode::DiskFull,..},_)) {
        "storage_pressure: the search index has no space left".into()
    } else {format!("Search index operation failed: {error}")}
}
fn check(cancel:&AtomicBool) -> Result<(),String> {if cancel.load(Ordering::Relaxed) {Err("read_canceled".into())} else {Ok(())}}
fn now() -> i64 {std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d|d.as_secs() as i64).unwrap_or(0)}

pub(crate) fn path(data_dir:&Path) -> PathBuf {data_dir.join("search-index").join("corpus.sqlite")}

/// `file:` URI for attaching the business database read-only.
fn read_only_uri(path:&str) -> String {
    let mut uri=String::from("file:");
    for ch in path.chars() {match ch {'%'=>uri.push_str("%25"),' '=>uri.push_str("%20"),'?'=>uri.push_str("%3f"),'#'=>uri.push_str("%23"),_=>uri.push(ch)}}
    uri.push_str("?mode=ro");uri
}

/// Open the index with the business database attached as `app`, rebuilding it when its version changed.
pub(super) fn open(app:&AppCtx) -> Result<Connection,String> {
    let file=path(&app.data_dir()?);
    std::fs::create_dir_all(file.parent().unwrap()).map_err(|_|"storage_pressure: the search index directory is unavailable")?;
    let conn=Connection::open(&file).map_err(sql)?;
    conn.busy_timeout(std::time::Duration::from_secs(30)).map_err(sql)?;
    // auto_vacuum must be chosen before the first table exists; it is a no-op on an existing file.
    conn.execute_batch("PRAGMA auto_vacuum=INCREMENTAL; PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;").map_err(sql)?;
    let version:Option<i64>=conn.query_row("SELECT version FROM corpus_meta",[],|r|r.get(0)).optional().unwrap_or(None);
    if version!=Some(VERSION) {
        conn.execute_batch("DROP TABLE IF EXISTS session_words; DROP TABLE IF EXISTS session_fts; DROP TABLE IF EXISTS corpus_state;
            DROP TABLE IF EXISTS corpus_meta;").map_err(sql)?;
        conn.execute_batch(SCHEMA).map_err(sql)?;
        conn.execute("INSERT INTO corpus_meta VALUES(?1)",[VERSION]).map_err(sql)?;
        history::store::reclaim_free_pages(&conn).map_err(sql)?;
    }
    let main=app.db().conn.lock().unwrap().path().map(str::to_owned).filter(|path|!path.is_empty())
        .ok_or("Search requires a file-backed application database")?;
    conn.execute("ATTACH DATABASE ?1 AS app",[read_only_uri(&main)]).map_err(sql)?;
    Ok(conn)
}

#[derive(Clone)]
pub(super) struct Session {pub id:String,pub name:String,pub kind:SessionKind,pub native:Option<String>,pub archived:bool}

/// Sessions in id order after `after`, 64 at a time, with every text field bounded before it is read.
pub(super) fn sessions(app:&AppCtx,after:Option<&str>) -> Result<Vec<Session>,String> {
    let conn=app.db().conn.lock().unwrap();
    let mut statement=conn.prepare("SELECT substr(id,1,128),substr(name,1,4096),kind,substr(agent_session_id,1,4096),archived_at IS NOT NULL,
        length(CAST(id AS BLOB))>128 OR length(CAST(name AS BLOB))>4096 OR length(CAST(agent_session_id AS BLOB))>4096
        FROM sessions WHERE (?1 IS NULL OR id>?1) ORDER BY id LIMIT 64").map_err(sql)?;
    let rows=statement.query_map([after],|r|Ok((Session {id:r.get(0)?,name:r.get(1)?,kind:SessionKind::from_db(&r.get::<_,String>(2)?),native:r.get(3)?,archived:r.get(4)?},r.get::<_,Option<bool>>(5)?.unwrap_or(false)))).map_err(sql)?;
    rows.map(|row|row.map_err(sql).and_then(|(session,large)|if large {Err("unsupported_record: session metadata exceeds its byte budget".into())} else {Ok(session)})).collect()
}

/// The indexed branch of a session's native recording, or None when it has none to read.
fn native(app:&AppCtx,session:&Session,path:&Path,cancel:&AtomicBool) -> Result<Option<Branch>,String> {
    let Some(id)=session.native.as_deref().map(str::trim).filter(|id|!id.is_empty()) else {return Ok(None);};
    if session.kind==SessionKind::Kiro {
        return match history::event_stream::complete(app,&session.id,cancel) {Ok(branch)=>Ok(Some(branch)),Err(error) if error.starts_with("source_missing:")=>Ok(None),Err(error)=>Err(error)};
    }
    let store=Store::at(&app.data_dir()?)?;
    let source=store.register(session.kind,history::source::Segment {path:path.into(),native_id:id.into(),end_byte:None,end_ordinal:None})?;
    let branch=history::api::bind(store,vec![source],session.kind,id.into(),None,&format!("global-search:{}",session.id))?;
    history::event_stream::wait(branch,cancel).map(Some)
}

#[derive(Clone,PartialEq)]
struct State {source:String,path:Option<String>,length:i64,modified:Option<i64>,generation:Option<String>,messages:i64,resume:Option<String>}

fn state(conn:&Connection,session:&str) -> Result<Option<State>,String> {
    conn.query_row("SELECT source,path,length,modified,generation,messages,resume FROM corpus_state WHERE session_id=?1",[session],
        |r|Ok(State {source:r.get(0)?,path:r.get(1)?,length:r.get(2)?,modified:r.get(3)?,generation:r.get(4)?,messages:r.get(5)?,resume:r.get(6)?})).optional().map_err(sql)
}
fn stat(path:&Path) -> Option<(i64,Option<i64>)> {
    let meta=std::fs::metadata(path).ok().filter(|meta|meta.is_file())?;
    let modified=meta.modified().ok().and_then(|time|time.duration_since(std::time::UNIX_EPOCH).ok()).map(|d|d.as_secs() as i64);
    Some((meta.len() as i64,modified))
}

/// Where a session's searchable text comes from, resolved with at most one directory lookup.
enum Track {Transcript(PathBuf),Recording(PathBuf),None}
fn track(app:&AppCtx,session:&Session,previous:Option<&State>) -> Result<Track,String> {
    if let Some(id)=session.native.as_deref().map(str::trim).filter(|id|!id.is_empty()) {
        if session.kind==SessionKind::Kiro {return Ok(Track::Transcript(PathBuf::new()));}
        // The recorded path costs one stat; the per-kind lookup walks the agent's whole session tree.
        let remembered=previous.filter(|state|state.source=="transcript" || state.source=="failed").and_then(|state|state.path.clone()).map(PathBuf::from).filter(|path|path.is_file());
        if let Some(path)=remembered.or_else(||transcript::source_path(session.kind,id).filter(|path|path.is_file())) {return Ok(Track::Transcript(path));}
        // A file already authorized for this session stays searchable after the provider's directory
        // catalogue changes. Only the recorded association and native identity are reused; protocol-only
        // sources are not this track.
        if matches!(session.kind,SessionKind::Claude|SessionKind::Codex|SessionKind::Grok|SessionKind::Pi|SessionKind::Omp|SessionKind::Antigravity) {
            let store=Store::at(&app.data_dir()?)?;
            let known:Option<String>=store.connect()?.query_row("SELECT s.path FROM sources s JOIN segments g ON g.source_id=s.id AND g.generation=s.generation
                WHERE g.session_scope IN (?1,?2) AND s.provider=?3 AND s.native_id=?4 AND length(CAST(s.path AS BLOB))<=4096
                AND NOT EXISTS(SELECT 1 FROM protocol_sources p WHERE p.source_id=s.id) ORDER BY g.sequence DESC LIMIT 1",
                params![session.id,format!("global-search:{}",session.id),session.kind.as_str(),id],|r|r.get(0)).optional().map_err(sql)?;
            if let Some(path)=known.map(PathBuf::from).filter(|path|path.is_file()) {return Ok(Track::Transcript(path));}
        }
    }
    let recording=app.data_dir()?.join("recordings").join(format!("{}.log",session.id));
    Ok(if recording.is_file() {Track::Recording(recording)} else {Track::None})
}

fn delete_rows(conn:&Connection,session:&str,from_message:Option<i64>) -> Result<(),String> {
    conn.execute("DELETE FROM session_words WHERE rowid IN (SELECT rowid FROM session_fts WHERE session_id=?1 AND (?2 IS NULL OR message_index>=?2))",
        params![session,from_message]).map_err(sql)?;
    conn.execute("DELETE FROM session_fts WHERE session_id=?1 AND (?2 IS NULL OR message_index>=?2)",params![session,from_message]).map_err(sql)?;Ok(())
}
pub(super) fn insert(conn:&Connection,session:&str,source:&str,message:Option<i64>,ordinal:i64,role:Option<&str>,ts:Option<&str>,text:&str) -> Result<(),String> {
    if !text.chars().any(|ch|!ch.is_whitespace()) {return Ok(());}
    conn.execute("INSERT INTO session_fts(text,session_id,source,message_index,ordinal,role,ts) VALUES(?1,?2,?3,?4,?5,?6,?7)",
        params![text,session,source,message,ordinal,role,ts]).map_err(sql)?;
    conn.execute("INSERT INTO session_words(rowid,words,session_id) VALUES(?1,?2,?3)",
        params![conn.last_insert_rowid(),tokenize::boundary_marked(text),session]).map_err(sql)?;Ok(())
}

/// Read `input` as UTF-8 text in blocks of at most [`CHUNK_BYTES`], ending a full block after its last
/// line break when one is near its end so a block usually ends between sentences.
fn chunks(input:&mut impl Read,cancel:&AtomicBool,mut each:impl FnMut(&str)->Result<(),String>) -> Result<(),String> {
    let mut pending:Vec<u8>=Vec::with_capacity(CHUNK_BYTES);let mut buffer=vec![0;CHUNK_BYTES];let mut done=false;
    loop {
        check(cancel)?;
        while pending.len()<CHUNK_BYTES && !done {
            let count=input.read(&mut buffer[..CHUNK_BYTES-pending.len()]).map_err(|e|e.to_string())?;
            if count==0 {done=true;} else {pending.extend_from_slice(&buffer[..count]);}
        }
        if pending.is_empty() {return Ok(());}
        let complete=match std::str::from_utf8(&pending) {Ok(text)=>text.len(),Err(error) if error.error_len().is_none() && !done=>error.valid_up_to(),
            Err(_)=>return Err("source_changed: search text is not valid UTF-8".into())};
        let mut end=complete;
        if !done || complete<pending.len() {
            let text=std::str::from_utf8(&pending[..complete]).unwrap();
            let from=ceil_char_boundary(text,complete.saturating_sub(4096));
            if let Some(at)=text[from..].rfind('\n').map(|at|from+at+1) {end=at;}
        }
        if end==0 {end=complete.max(1);}
        each(std::str::from_utf8(&pending[..end]).map_err(|_|"source_changed: search text is not valid UTF-8")?)?;
        pending.drain(..end);
        if done && pending.is_empty() {return Ok(());}
    }
}

/// Index the messages of `prose` from `start` on, replacing the session's rows from that message.
fn index_prose(conn:&Connection,session:&str,prose:&Rc<Prose>,start:i64,cancel:&Arc<AtomicBool>) -> Result<(),String> {
    delete_rows(conn,session,(start>0).then_some(start))?;
    for number in start.max(0) as u64..prose.total {
        check(cancel)?;
        let metadata=prose.metadata(number)?.ok_or("source_changed: original message metadata is unavailable")?;
        let (role,ts)=(metadata["role"].as_str(),metadata["timestamp"].as_str());
        let mut reader=Reader::new(prose.clone(),number,cancel.clone())?;let mut part=0i64;
        chunks(&mut reader,cancel,|text| {
            insert(conn,session,"transcript",Some(number as i64),((number as i64)<<20)+part,role,ts,text)?;part+=1;Ok(())
        })?;
    }
    Ok(())
}

/// Index every visible line of a terminal recording.
fn index_recording(conn:&Connection,session:&str,path:&Path,cancel:&AtomicBool) -> Result<(),String> {
    delete_rows(conn,session,None)?;
    let mut file=std::fs::File::open(path).map_err(|_|"source_missing: recording is unavailable")?;
    let mut stripper=super::ansi::AnsiStripper::new();let mut ordinal=0i64;let mut failed=None;
    let mut line=|text:&str| {
        if failed.is_some() {return;}
        for piece in text.as_bytes().chunks(CHUNK_BYTES) {
            let piece=String::from_utf8_lossy(piece);
            if let Err(error)=insert(conn,session,"recording",None,ordinal,None,None,piece.trim()) {failed=Some(error);return;}
        }
        if text.chars().any(|ch|!ch.is_whitespace()) {ordinal+=1;}
    };
    let mut bytes=vec![0;CHUNK_BYTES];
    loop {
        check(cancel)?;let count=file.read(&mut bytes).map_err(|e|e.to_string())?;if count==0 {break;}
        stripper.feed(&bytes[..count],&mut line);
    }
    stripper.finish(&mut line);
    failed.map_or(Ok(()),Err)
}

/// One refresher at a time per process: a search and the startup warm-up interleave session by session.
fn gate() -> &'static Mutex<()> {static GATE:OnceLock<Mutex<()>>=OnceLock::new();GATE.get_or_init(||Mutex::new(()))}

/// What a refresh did; `Indexed` carries the source size that was read.
pub(super) enum Refreshed {Unchanged,Indexed(u64),Removed}

/// Bring one session's rows up to date with its source.
pub(super) fn refresh_session(app:&AppCtx,conn:&Connection,session:&Session,cancel:&Arc<AtomicBool>) -> Result<Refreshed,String> {
    let _gate=gate().lock().unwrap_or_else(|poisoned|poisoned.into_inner());
    check(cancel)?;
    let previous=state(conn,&session.id)?;
    let track=track(app,session,previous.as_ref())?;
    let (source,path)=match &track {Track::Transcript(path)=>("transcript",path.clone()),Track::Recording(path)=>("recording",path.clone()),Track::None=> {
        if previous.is_none() {return Ok(Refreshed::Unchanged);}
        conn.execute_batch("BEGIN IMMEDIATE").map_err(sql)?;
        let result=delete_rows(conn,&session.id,None).and_then(|_|conn.execute("DELETE FROM corpus_state WHERE session_id=?1",[&session.id]).map(|_|()).map_err(sql));
        return finish(conn,result).map(|_|Refreshed::Removed);
    }};
    let kiro=session.kind==SessionKind::Kiro;
    let (length,modified)=if kiro {(0,None)} else {stat(&path).ok_or("source_missing: search source is unavailable")?};
    let path_text=(!kiro).then(||path.to_string_lossy().into_owned());
    if !kiro && previous.as_ref().is_some_and(|state|state.source==source && state.path==path_text && state.length==length && state.modified==modified) {
        return Ok(Refreshed::Unchanged);
    }
    if source=="recording" {
        conn.execute_batch("BEGIN IMMEDIATE").map_err(sql)?;
        let result=index_recording(conn,&session.id,&path,cancel).and_then(|_|write_state(conn,&session.id,&State {
            source:source.into(),path:path_text,length,modified,generation:None,messages:0,resume:None}));
        return finish(conn,result).map(|_|Refreshed::Indexed(length as u64));
    }
    // A recording that failed for what it contains fails the same way until it changes.
    if !kiro && previous.as_ref().is_some_and(|state|state.source=="failed" && state.path==path_text && state.length==length && state.modified==modified) {
        return Ok(Refreshed::Unchanged);
    }
    let result=index_transcript(app,conn,session,&path,previous.as_ref(),(path_text.clone(),length,modified),cancel);
    if let Err(error)=&result {
        if ["index_incomplete:","unsupported_record:","detail_unavailable:"].iter().any(|code|error.starts_with(code)) {
            write_state(conn,&session.id,&State {source:"failed".into(),path:path_text,length,modified,generation:None,messages:0,resume:None})?;
        }
    }
    result
}

fn index_transcript(app:&AppCtx,conn:&Connection,session:&Session,path:&Path,previous:Option<&State>,(path_text,length,modified):(Option<String>,i64,Option<i64>),
    cancel:&Arc<AtomicBool>) -> Result<Refreshed,String> {
    let kiro=session.kind==SessionKind::Kiro;
    let Some(branch)=native(app,session,path,cancel)? else {return Ok(Refreshed::Unchanged);};
    let generation=branch.generation.clone();
    if kiro && previous.is_some_and(|state|state.generation.as_deref()==Some(generation.as_str())) {return Ok(Refreshed::Unchanged);}
    // A conversation that only grew is read again from where its last indexed message began; the
    // last message may itself have grown. The resume point checks that nothing before it changed,
    // which also holds after the history index re-read the recording under a new generation.
    let grown=previous.filter(|state|state.source=="transcript" && state.path==path_text && length>=state.length);
    let resume=grown.and_then(|state|state.resume.as_deref()).and_then(|raw|serde_json::from_str::<serde_json::Value>(raw).ok())
        .and_then(|value|Some(Start {source:value["source"].as_str()?.into(),record:value["record"].as_u64()?,piece:value["piece"].as_u64()?,
            number:value["number"].as_u64()?,identity:value["identity"].as_str()?.into(),before:value["before"].as_u64()?}))
        .filter(|start|grown.is_some_and(|state|start.number as i64==state.messages-1));
    let resumed=match &resume {Some(start)=>Prose::resume(branch.clone(),cancel,start)?,None=>None};
    let (prose,start)=match (resumed,&resume) {
        (Some(prose),Some(resume))=>(prose,resume.number as i64),
        _=>{
            let prose=Prose::new(branch,cancel)?;
            // Without a resume point an unchanged generation still proves the earlier messages unchanged.
            let start=match grown {
                Some(state) if state.generation.as_deref()==Some(generation.as_str()) && prose.total as i64>=state.messages => state.messages.saturating_sub(1).max(0),
                _=>0,
            };
            (prose,start)
        },
    };
    let prose=Rc::new(prose);
    let resume=match prose.total.checked_sub(1) {Some(last)=>prose.start(last)?,None=>None}
        .map(|start|serde_json::json!({"source":start.source,"record":start.record,"piece":start.piece,"number":start.number,
            "identity":start.identity,"before":start.before}).to_string());
    conn.execute_batch("BEGIN IMMEDIATE").map_err(sql)?;
    let result=index_prose(conn,&session.id,&prose,start,cancel).and_then(|_|write_state(conn,&session.id,&State {
        source:"transcript".into(),path:path_text,length,modified,generation:Some(generation),messages:prose.total as i64,resume}));
    finish(conn,result).map(|_|Refreshed::Indexed(length as u64))
}

fn write_state(conn:&Connection,session:&str,state:&State) -> Result<(),String> {
    conn.execute("INSERT OR REPLACE INTO corpus_state VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
        params![session,state.source,state.path,state.length,state.modified,state.generation,state.messages,now(),state.resume]).map_err(sql)?;Ok(())
}
fn finish(conn:&Connection,result:Result<(),String>) -> Result<(),String> {
    match result {
        Ok(())=>{conn.execute_batch("COMMIT").map_err(sql)?;history::store::reclaim_free_pages(conn).map_err(sql)},
        Err(error)=>{let _=conn.execute_batch("ROLLBACK");Err(error)},
    }
}

/// Remove rows of sessions that no longer exist in the business database.
pub(super) fn prune(conn:&Connection) -> Result<(),String> {
    conn.execute_batch("BEGIN IMMEDIATE;
        DELETE FROM session_words WHERE session_id NOT IN (SELECT id FROM app.sessions);
        DELETE FROM session_fts WHERE session_id NOT IN (SELECT id FROM app.sessions);
        DELETE FROM corpus_state WHERE session_id NOT IN (SELECT id FROM app.sessions);
        COMMIT;").map_err(sql)?;
    history::store::reclaim_free_pages(conn).map_err(sql)
}

/// Delete removed sessions' rows right away rather than at the next prune.
pub fn drop_sessions(app:&AppCtx,ids:&[String]) -> Result<(),String> {
    if ids.is_empty() || !path(&app.data_dir()?).is_file() {return Ok(());}
    let conn=open(app)?;conn.execute_batch("BEGIN IMMEDIATE").map_err(sql)?;
    let result=ids.iter().try_for_each(|id| {
        delete_rows(&conn,id,None)?;conn.execute("DELETE FROM corpus_state WHERE session_id=?1",[id]).map(|_|()).map_err(sql)
    });
    finish(&conn,result)
}

/// Merge fragmented FTS segments after many updates so the file stays compact.
pub(super) fn optimize(conn:&Connection) -> Result<(),String> {
    conn.execute_batch("INSERT INTO session_fts(session_fts) VALUES('optimize'); INSERT INTO session_words(session_words) VALUES('optimize');").map_err(sql)?;
    history::store::reclaim_free_pages(conn).map_err(sql)
}

// ─────────────────────────── Query ───────────────────────────

/// Which index answered the query; it decides snippet placement, matched literals and ordering.
#[derive(Clone,Copy,PartialEq,Eq,Debug)]
pub(super) enum Answered {Words,Trigram,Like}
impl Answered {pub(super) fn code(self) -> usize {match self {Self::Words=>0,Self::Trigram=>1,Self::Like=>2}}}

/// A matching session in result order, without its snippets.
#[derive(Clone,Debug)]
pub(super) struct Ranked {pub session:String,pub count:u32,pub source:String}

pub(super) struct Query {pub keys:Vec<String>,pub answered:Answered,expression:String}

fn scope_filter(scope:SearchScope) -> &'static str {
    match scope {SearchScope::All=>"",SearchScope::Live=>" AND s.archived_at IS NULL",SearchScope::Archived=>" AND s.archived_at IS NOT NULL"}
}

fn fts_match_expr(keys:&[String]) -> String {
    keys.iter().map(|k|format!("\"{}\"",k.replace('"',"\"\""))).collect::<Vec<_>>().join(" ")
}
fn like_escape(k:&str) -> String {
    let mut out=String::with_capacity(k.len());
    for c in k.chars() {if c=='\\' || c=='%' || c=='_' {out.push('\\');}out.push(c);}out
}

/// Matching sessions in relevance order. The word index answers first; the trigram index or, for
/// terms shorter than three characters, LIKE answers only when no word matched.
pub(super) fn rank(conn:&Connection,query:&str,scope:SearchScope,session:Option<&str>) -> Result<(Option<Query>,Vec<Ranked>),String> {
    let keys:Vec<String>=query.split_whitespace().map(str::to_string).collect();
    if keys.is_empty() {return Ok((None,Vec::new()));}
    let filter=format!("{}{}",scope_filter(scope),if session.is_some() {" AND f.session_id=?2"} else {""});
    let run=|sql:String,binds:Vec<String>| -> Result<Vec<Ranked>,String> {
        let mut statement=conn.prepare(&sql).map_err(sql_error)?;
        let rows=statement.query_map(params_from_iter(binds.iter()),|r|Ok(Ranked {session:r.get(0)?,count:r.get(1)?,source:r.get(2)?})).map_err(sql_error)?;
        rows.collect::<Result<Vec<_>,_>>().map_err(sql_error)
    };
    let with_session=|first:String| -> Vec<String> {let mut binds=vec![first];if let Some(id)=session {binds.push(id.into());}binds};
    if let Some(phrases)=keys.iter().map(|k|tokenize::query_phrase(k)).collect::<Option<Vec<_>>>() {
        let expression=phrases.join(" ");
        let ranked=run(format!("SELECT f.session_id,count(*),min(f.source) FROM session_words w JOIN session_fts f ON f.rowid=w.rowid
            JOIN app.sessions s ON s.id=f.session_id WHERE session_words MATCH ?1{filter} GROUP BY f.session_id ORDER BY min(w.rank),f.session_id"),with_session(expression.clone()))?;
        if !ranked.is_empty() {return Ok((Some(Query {keys,answered:Answered::Words,expression}),ranked));}
    }
    if keys.iter().all(|k|k.chars().count()>=MIN_FTS_TERM_CHARS) {
        let expression=fts_match_expr(&keys);
        let ranked=run(format!("SELECT f.session_id,count(*),min(f.source) FROM session_fts f JOIN app.sessions s ON s.id=f.session_id
            WHERE session_fts MATCH ?1{filter} GROUP BY f.session_id ORDER BY min(f.rank),f.session_id"),with_session(expression.clone()))?;
        return Ok((Some(Query {keys,answered:Answered::Trigram,expression}),ranked));
    }
    let clause=keys.iter().enumerate().map(|(index,_)|format!("f.text LIKE ?{} ESCAPE '\\'",index+2)).collect::<Vec<_>>().join(" AND ");
    let like_filter=format!("{}{}",scope_filter(scope),if session.is_some() {" AND f.session_id=?1"} else {""});
    let mut binds=vec![session.unwrap_or_default().to_owned()];binds.extend(keys.iter().map(|k|format!("%{}%",like_escape(k))));
    let mut ranked=run(format!("SELECT f.session_id,count(*),min(f.source) FROM session_fts f JOIN app.sessions s ON s.id=f.session_id
        WHERE {clause}{like_filter} GROUP BY f.session_id ORDER BY count(*) DESC,f.session_id"),binds)?;
    ranked.sort_by(|a,b|b.count.cmp(&a.count));
    Ok((Some(Query {keys,answered:Answered::Like,expression:String::new()}),ranked))
}
fn sql_error(error:rusqlite::Error) -> String {sql(error)}

/// Up to [`MAX_MATCHES`] snippets for one ranked session, in document order.
pub(super) fn matches(conn:&Connection,query:&Query,session:&str) -> Result<Vec<SearchMatch>,String> {
    struct Raw {message:Option<i64>,text:String,spans:Vec<(usize,usize)>,role:Option<String>,ts:Option<String>,transcript:bool}
    let limit=MAX_MATCHES as i64;
    let raws:Vec<Raw>=match query.answered {
        Answered::Words=> {
            let mut statement=conn.prepare("SELECT f.message_index,highlight(session_words,0,?2,?3),f.role,f.ts,f.source FROM session_words w JOIN session_fts f ON f.rowid=w.rowid
                WHERE session_words MATCH ?1 AND f.session_id=?4 ORDER BY f.ordinal LIMIT ?5").map_err(sql)?;
            let rows=statement.query_map(params![query.expression,tokenize::MARK_OPEN,tokenize::MARK_CLOSE,session,limit],|r| {
                let marked:String=r.get(1)?;let highlighted=tokenize::parse_highlight(&marked);
                Ok(Raw {message:r.get(0)?,text:highlighted.text,spans:highlighted.spans,role:r.get(2)?,ts:r.get(3)?,transcript:r.get::<_,String>(4)?=="transcript"})
            }).map_err(sql)?;rows.collect::<Result<_,_>>().map_err(sql)?
        },
        Answered::Trigram|Answered::Like=> {
            let (condition,binds)=if query.answered==Answered::Trigram {("session_fts MATCH ?1".to_owned(),vec![query.expression.clone()])} else {
                (query.keys.iter().enumerate().map(|(index,_)|format!("f.text LIKE ?{} ESCAPE '\\'",index+1)).collect::<Vec<_>>().join(" AND "),
                    query.keys.iter().map(|k|format!("%{}%",like_escape(k))).collect())
            };
            let at=binds.len();
            let mut statement=conn.prepare(&format!("SELECT f.message_index,f.text,f.role,f.ts,f.source FROM session_fts f WHERE {condition}
                AND f.session_id=?{} ORDER BY f.ordinal LIMIT ?{}",at+1,at+2)).map_err(sql)?;
            let mut values:Vec<rusqlite::types::Value>=binds.into_iter().map(rusqlite::types::Value::Text).collect();
            values.push(rusqlite::types::Value::Text(session.into()));values.push(rusqlite::types::Value::Integer(limit));
            let rows=statement.query_map(params_from_iter(values.iter()),|r|Ok(Raw {message:r.get(0)?,text:r.get(1)?,spans:Vec::new(),role:r.get(2)?,ts:r.get(3)?,
                transcript:r.get::<_,String>(4)?=="transcript"})).map_err(sql)?;rows.collect::<Result<_,_>>().map_err(sql)?
        },
    };
    Ok(raws.into_iter().enumerate().map(|(index,raw)| {
        let (snippet,matched)=if query.answered==Answered::Words {
            let snippet=match (raw.transcript,raw.spans.first()) {(true,Some(&(at,end)))=>snippet_around(&raw.text,at,end-at),_=>clip_line(&raw.text)};
            (snippet,matched_literals(&raw.text,&raw.spans))
        } else {(if raw.transcript {snippet_for(&raw.text,&query.keys)} else {clip_line(&raw.text)},query.keys.clone())};
        SearchMatch {message_index:raw.message.map(|v|v as u32),ordinal:(index+1) as u32,snippet,matched,role:raw.role,timestamp:raw.ts,row_id:None}
    }).collect())
}

/// The complete hit for one ranked session, with names read from the business database.
pub(super) fn hit(conn:&Connection,query:&Query,ranked:&Ranked,names:&HashMap<String,(String,SessionKind,bool)>) -> Result<Option<SessionSearchHit>,String> {
    let Some((name,kind,archived))=names.get(&ranked.session) else {return Ok(None);};
    Ok(Some(SessionSearchHit {session_id:ranked.session.clone(),name:name.clone(),kind:*kind,archived:*archived,
        source:if ranked.source=="transcript" {"transcript"} else {"recording"},match_count:ranked.count,matches:matches(conn,query,&ranked.session)?}))
}

fn matched_literals(text:&str,spans:&[(usize,usize)]) -> Vec<String> {
    let mut out:Vec<String>=Vec::new();
    for &(a,b) in spans {let literal=&text[a..b];if !out.iter().any(|m|m==literal) {out.push(literal.to_string());}}out
}
fn snippet_for(text:&str,keys:&[String]) -> String {
    let lower=text.to_lowercase();let mut best:Option<(usize,usize)>=None;
    for k in keys {
        let kl=k.to_lowercase();if kl.is_empty() {continue;}
        if let Some(p)=lower.find(&kl) {if best.is_none_or(|(bp,_)|p<bp) {best=Some((p,kl.len()));}}
    }
    // Lowercasing can change byte lengths; snippet_around clamps to character boundaries either way.
    match best {Some((at,len))=>snippet_around(text,at,len),None=>clip_line(text)}
}
fn snippet_around(text:&str,at:usize,hit_len:usize) -> String {
    let len=text.len();let at=at.min(len);
    let start=floor_char_boundary(text,at.saturating_sub(SNIPPET_CONTEXT));
    let end=ceil_char_boundary(text,(at+hit_len+SNIPPET_CONTEXT).min(len));
    let core=text[start..end].replace(['\n','\r','\t']," ");let mut out=String::new();
    if start>0 {out.push('…');}out.push_str(core.trim());if end<len {out.push('…');}
    limit_chars(&out)
}
fn clip_line(line:&str) -> String {limit_chars(line.trim())}
fn floor_char_boundary(s:&str,mut i:usize) -> usize {if i>=s.len() {return s.len();}while i>0 && !s.is_char_boundary(i) {i-=1;}i}
fn ceil_char_boundary(s:&str,mut i:usize) -> usize {if i>=s.len() {return s.len();}while i<s.len() && !s.is_char_boundary(i) {i+=1;}i}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn message(id:&str,role:&str,text:&str) -> String {
        format!("{}\n",serde_json::json!({"type":"response_item","payload":{"type":"message","id":id,"role":role,
            "content":[{"type":if role=="user" {"input_text"} else {"output_text"},"text":text}]}}))
    }

    #[test]
    fn transcript_appends_index_only_new_messages_and_unchanged_sources_cost_one_stat() {
        let root=std::env::temp_dir().join(format!("vlx-search-corpus-{}",uuid::Uuid::new_v4()));std::fs::create_dir_all(&root).unwrap();
        let db=crate::db::Db::open(&root.join("business.sqlite")).unwrap();
        db.conn.lock().unwrap().execute_batch("INSERT INTO projects(id,name,root_path,sort_order,created_at) VALUES('p','p','/p',0,0);
            INSERT INTO sessions(id,project_id,name,kind,sort_order,created_at,agent_session_id) VALUES('s','p','codex','codex',0,0,'thread');").unwrap();
        let app=AppCtx::Headless(Arc::new(crate::host::HeadlessHost::new(root.clone(),db)));
        let rollout=root.join("rollout-thread.jsonl");
        std::fs::write(&rollout,format!("{}{}",message("u1","user","Where is the alpha config?"),message("a1","assistant","The alpha config lives in settings."))).unwrap();
        let conn=open(&app).unwrap();
        // The provider's directory lookup is not part of this test: start from a remembered path.
        conn.execute("INSERT INTO corpus_state VALUES('s','transcript',?1,0,NULL,NULL,0,0,NULL)",[rollout.to_string_lossy()]).unwrap();
        let session=sessions(&app,None).unwrap().remove(0);let cancel=Arc::new(AtomicBool::new(false));
        let rows=|conn:&Connection| conn.query_row("SELECT count(*),count(DISTINCT message_index) FROM session_fts WHERE session_id='s'",[],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?))).unwrap();
        assert!(matches!(refresh_session(&app,&conn,&session,&cancel).unwrap(),Refreshed::Indexed(_)));
        assert_eq!(rows(&conn),(2,2));
        assert!(matches!(refresh_session(&app,&conn,&session,&cancel).unwrap(),Refreshed::Unchanged));
        let (query,ranked)=rank(&conn,"alpha",SearchScope::All,None).unwrap();assert_eq!(ranked.len(),1);assert_eq!(ranked[0].count,2);
        let found=matches(&conn,&query.unwrap(),"s").unwrap();assert_eq!(found[0].message_index,Some(0));assert_eq!(found[1].message_index,Some(1));
        // An append keeps earlier rows and adds only the new message.
        std::fs::OpenOptions::new().append(true).open(&rollout).unwrap().write_all(message("u2","user","And the omega one?").as_bytes()).unwrap();
        let later=std::time::SystemTime::now()+std::time::Duration::from_secs(2);std::fs::File::options().write(true).open(&rollout).unwrap().set_modified(later).unwrap();
        assert!(matches!(refresh_session(&app,&conn,&session,&cancel).unwrap(),Refreshed::Indexed(_)));
        assert_eq!(rows(&conn),(3,3));
        assert_eq!(rank(&conn,"omega",SearchScope::All,None).unwrap().1.len(),1);
        // A long message becomes several fragments of the same message.
        std::fs::OpenOptions::new().append(true).open(&rollout).unwrap().write_all(message("a2","assistant",&"long answer line\n".repeat(9000)).as_bytes()).unwrap();
        let later=std::time::SystemTime::now()+std::time::Duration::from_secs(4);std::fs::File::options().write(true).open(&rollout).unwrap().set_modified(later).unwrap();
        refresh_session(&app,&conn,&session,&cancel).unwrap();
        let (fragments,messages)=rows(&conn);assert_eq!(messages,4);assert!(fragments>=5,"{fragments}");
        // Removing the session from the business database prunes its rows.
        app.db().conn.lock().unwrap().execute("DELETE FROM sessions WHERE id='s'",[]).unwrap();prune(&conn).unwrap();assert_eq!(rows(&conn),(0,0));
        drop(conn);crate::agent::history_store::store::release_test_directory(&root);drop(app);let _=std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_appended_conversation_resumes_at_its_last_message_and_matches_a_full_rebuild() {
        let root=std::env::temp_dir().join(format!("vlx-search-resume-{}",uuid::Uuid::new_v4()));std::fs::create_dir_all(&root).unwrap();
        let db=crate::db::Db::open(&root.join("business.sqlite")).unwrap();
        db.conn.lock().unwrap().execute_batch("INSERT INTO projects(id,name,root_path,sort_order,created_at) VALUES('p','p','/p',0,0);
            INSERT INTO sessions(id,project_id,name,kind,sort_order,created_at,agent_session_id) VALUES('s','p','codex','codex',0,0,'thread');").unwrap();
        let app=AppCtx::Headless(Arc::new(crate::host::HeadlessHost::new(root.clone(),db)));
        let rollout=root.join("rollout-thread.jsonl");std::fs::write(&rollout,message("u1","user","First question")).unwrap();
        let conn=open(&app).unwrap();
        conn.execute("INSERT INTO corpus_state VALUES('s','transcript',?1,0,NULL,NULL,0,0,NULL)",[rollout.to_string_lossy()]).unwrap();
        let session=sessions(&app,None).unwrap().remove(0);let cancel=Arc::new(AtomicBool::new(false));
        let rows=|conn:&Connection| {
            let mut statement=conn.prepare("SELECT message_index,group_concat(text,'|') FROM (SELECT message_index,text FROM session_fts WHERE session_id='s' ORDER BY ordinal) GROUP BY message_index ORDER BY message_index").unwrap();
            statement.query_map([],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,String>(1)?))).unwrap().collect::<Result<Vec<_>,_>>().unwrap()
        };
        refresh_session(&app,&conn,&session,&cancel).unwrap();
        let appends=[message("a1","assistant","First answer"),message("a2","assistant","continued in the same message"),
            message("u2","user","Second question")+&message("a3","assistant","Second answer")];
        for (index,append) in appends.iter().enumerate() {
            std::fs::OpenOptions::new().append(true).open(&rollout).unwrap().write_all(append.as_bytes()).unwrap();
            let later=std::time::SystemTime::now()+std::time::Duration::from_secs(2*(index as u64+1));
            std::fs::File::options().write(true).open(&rollout).unwrap().set_modified(later).unwrap();
            refresh_session(&app,&conn,&session,&cancel).unwrap();
            let resume:Option<String>=conn.query_row("SELECT resume FROM corpus_state WHERE session_id='s'",[],|r|r.get(0)).unwrap();assert!(resume.is_some());
        }
        // The history index is rebuilt from nothing (as after eviction): the next append still resumes.
        crate::agent::history_store::store::release_test_directory(&root);std::fs::remove_dir_all(root.join("chat-index")).unwrap();
        std::fs::OpenOptions::new().append(true).open(&rollout).unwrap().write_all(message("a4","assistant","after the rebuild").as_bytes()).unwrap();
        let later=std::time::SystemTime::now()+std::time::Duration::from_secs(20);std::fs::File::options().write(true).open(&rollout).unwrap().set_modified(later).unwrap();
        refresh_session(&app,&conn,&session,&cancel).unwrap();
        let incremental=rows(&conn);
        assert_eq!(incremental.len(),4,"{incremental:?}");assert!(incremental[3].1.contains("after the rebuild"),"{incremental:?}");assert!(incremental[1].1.contains("continued in the same message"),"{incremental:?}");
        // A rebuild from nothing gives the same rows.
        conn.execute("DELETE FROM corpus_state WHERE session_id='s'",[]).unwrap();delete_rows(&conn,"s",None).unwrap();
        conn.execute("INSERT INTO corpus_state VALUES('s','transcript',?1,0,NULL,NULL,0,0,NULL)",[rollout.to_string_lossy()]).unwrap();
        refresh_session(&app,&conn,&session,&cancel).unwrap();assert_eq!(rows(&conn),incremental);
        drop(conn);crate::agent::history_store::store::release_test_directory(&root);drop(app);let _=std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_conversation_that_cannot_be_read_is_not_read_again_until_it_changes() {
        let root=std::env::temp_dir().join(format!("vlx-search-failed-{}",uuid::Uuid::new_v4()));std::fs::create_dir_all(&root).unwrap();
        let db=crate::db::Db::open(&root.join("business.sqlite")).unwrap();
        db.conn.lock().unwrap().execute_batch("INSERT INTO projects(id,name,root_path,sort_order,created_at) VALUES('p','p','/p',0,0);
            INSERT INTO sessions(id,project_id,name,kind,sort_order,created_at,agent_session_id) VALUES('s','p','claude','claude',0,0,'thread');").unwrap();
        let app=AppCtx::Headless(Arc::new(crate::host::HeadlessHost::new(root.clone(),db)));
        // The only message names a parent the recording does not contain.
        let transcript=root.join("thread.jsonl");
        std::fs::write(&transcript,serde_json::json!({"type":"user","uuid":"b","parentUuid":"missing","message":{"role":"user","content":"Orphan"}}).to_string()+"\n").unwrap();
        let conn=open(&app).unwrap();
        conn.execute("INSERT INTO corpus_state VALUES('s','transcript',?1,0,NULL,NULL,0,0,NULL)",[transcript.to_string_lossy()]).unwrap();
        let session=sessions(&app,None).unwrap().remove(0);let cancel=Arc::new(AtomicBool::new(false));
        assert!(refresh_session(&app,&conn,&session,&cancel).err().unwrap().starts_with("index_incomplete:"));
        let source=|conn:&Connection| conn.query_row("SELECT source FROM corpus_state WHERE session_id='s'",[],|r|r.get::<_,String>(0)).unwrap();
        assert_eq!(source(&conn),"failed");
        assert!(matches!(refresh_session(&app,&conn,&session,&cancel).unwrap(),Refreshed::Unchanged),"an unchanged failure is skipped");
        // Once the recording changes it is read again.
        std::fs::write(&transcript,serde_json::json!({"type":"user","uuid":"a","message":{"role":"user","content":"Root question"}}).to_string()+"\n").unwrap();
        let later=std::time::SystemTime::now()+std::time::Duration::from_secs(2);std::fs::File::options().write(true).open(&transcript).unwrap().set_modified(later).unwrap();
        assert!(matches!(refresh_session(&app,&conn,&session,&cancel).unwrap(),Refreshed::Indexed(_)));assert_eq!(source(&conn),"transcript");
        drop(conn);crate::agent::history_store::store::release_test_directory(&root);drop(app);let _=std::fs::remove_dir_all(&root);
    }

    #[test]
    fn blocks_of_multibyte_text_without_line_breaks_split_on_character_boundaries() {
        let text="、".repeat(30_000)+"\n"+&"雪".repeat(30_000);let mut joined=String::new();let mut blocks=0;
        chunks(&mut text.as_bytes(),&AtomicBool::new(false),|block|{assert!(block.len()<=CHUNK_BYTES);joined.push_str(block);blocks+=1;Ok(())}).unwrap();
        assert_eq!(joined,text);assert!(blocks>=3);
    }

    /// Times a cold warm-up phase by phase on a copied data directory: VLX_BENCH_DATA=<dir>, optionally
    /// starting after VLX_BENCH_AFTER=<session id> for at most VLX_BENCH_LIMIT=<n> sessions.
    #[test]
    #[ignore]
    fn bench_cold_refresh_of_a_real_data_directory() {
        let Ok(dir)=std::env::var("VLX_BENCH_DATA") else {return;};let root=PathBuf::from(dir);
        let app=AppCtx::Headless(Arc::new(crate::host::HeadlessHost::new(root.clone(),crate::db::Db::open(&root.join("vlx-term.db")).unwrap())));
        let conn=open(&app).unwrap();let cancel=Arc::new(AtomicBool::new(false));
        // Writes a few pages and reports a reader left open by the finished session.
        let probe=|stage:&str,session:&str| {
            let Ok(index)=Connection::open(root.join("chat-index/index.sqlite")) else {return;};
            let _=index.execute_batch("CREATE TABLE IF NOT EXISTS bench_probe(v BLOB); INSERT INTO bench_probe VALUES(zeroblob(40000)); DELETE FROM bench_probe;");
            let (frames,copied):(i64,i64)=index.query_row("PRAGMA wal_checkpoint(PASSIVE)",[],|r|Ok((r.get(1)?,r.get(2)?))).unwrap_or((-1,-1));
            if frames!=copied {eprintln!("pinned after {stage} {session} {frames} {copied}");}
        };
        if std::env::var("VLX_BENCH_WARM").is_ok() {
            // A warm index: time what one search's refresh pass costs, session by session.
            let began=std::time::Instant::now();let mut after=None;
            loop {
                let batch=sessions(&app,after.as_deref()).unwrap();if batch.is_empty() {break;}after=batch.last().map(|s|s.id.clone());
                for session in batch {
                    let previous=state(&conn,&session.id).unwrap().map(|state|state.source);let t=std::time::Instant::now();
                    let result=refresh_session(&app,&conn,&session,&cancel);let spent=t.elapsed().as_secs_f64();
                    if spent>0.2 || matches!(result,Ok(Refreshed::Indexed(_))) {
                        let read=match &result {Ok(Refreshed::Indexed(bytes))=>format!("{:.1} MB",*bytes as f64/1e6),Ok(_)=>"unchanged".into(),Err(e)=>e.clone()};
                        eprintln!("refresh {} {:?} {previous:?} {spent:.2}s {read}",session.id,session.kind);
                    }
                }
            }
            eprintln!("refresh pass {:.1}s",began.elapsed().as_secs_f64());return;
        }
        let (mut scan,mut prose,mut write,mut bytes)=(0f64,0f64,0f64,0u64);let began=std::time::Instant::now();let mut after=std::env::var("VLX_BENCH_AFTER").ok();
        let limit=std::env::var("VLX_BENCH_LIMIT").ok().and_then(|v|v.parse::<usize>().ok()).unwrap_or(usize::MAX);let mut seen=0;
        loop {
            let batch=sessions(&app,after.as_deref()).unwrap();if batch.is_empty() || seen>=limit {break;}after=batch.last().map(|s|s.id.clone());
            for session in batch {
                seen+=1;if seen>limit {break;}
                let path=match track(&app,&session,None) {Ok(Track::Transcript(path))=>path,Ok(_)=>{eprintln!("untracked {}",session.id);continue;},Err(e)=>{eprintln!("skip {} {e}",session.id);continue;}};
                if session.kind==SessionKind::Kiro {continue;}
                let size=stat(&path).map(|s|s.0 as u64).unwrap_or(0);
                let t=std::time::Instant::now();let branch=match native(&app,&session,&path,&cancel) {Ok(Some(branch))=>branch,other=>{eprintln!("skip {} {:?}",session.id,other.err());continue;}};
                let a=t.elapsed().as_secs_f64();let t=std::time::Instant::now();
                let p=match Prose::new(branch,&cancel) {Ok(p)=>Rc::new(p),Err(e)=>{eprintln!("skip {} {e}",session.id);continue;}};
                let b=t.elapsed().as_secs_f64();let t=std::time::Instant::now();
                conn.execute_batch("BEGIN IMMEDIATE").unwrap();let result=index_prose(&conn,&session.id,&p,0,&cancel);if let Err(e)=finish(&conn,result) {eprintln!("skip {:?} {} {:.1} MB {e}",session.kind,session.id,size as f64/1e6);continue;}
                let c=t.elapsed().as_secs_f64();drop(p);probe("drop",&session.id);
                scan+=a;prose+=b;write+=c;bytes+=size;
                if a+b+c>1.0 {eprintln!("{} {:?} {:>7.1} MB scan {a:>6.1}s prose {b:>6.1}s write {c:>6.1}s",session.id,session.kind,size as f64/1e6);}
            }
        }
        eprintln!("total {:.1} MB in {:.0}s: scan {scan:.0}s prose {prose:.0}s write {write:.0}s",bytes as f64/1e6,began.elapsed().as_secs_f64());
    }
}


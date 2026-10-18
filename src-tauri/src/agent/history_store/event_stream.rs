//! Fixed-watermark event iteration for full-content tasks. Bodies remain verified field streams.

use std::collections::VecDeque;
use std::io::Write;
use std::sync::atomic::{AtomicBool,Ordering};

use rusqlite::{params,OptionalExtension};
use serde_json::Value;

use crate::{agent::export::{self,Event},host::AppCtx,models::SessionKind};
use super::{api::{self,Branch},projection::{Field,Record},store::{sql,RegisteredSource}};

pub fn complete(app:&AppCtx,session:&str,cancel:&AtomicBool) -> Result<Branch,String> {
    wait(api::prepare(app,session)?,cancel)
}
pub fn wait(mut branch:Branch,cancel:&AtomicBool) -> Result<Branch,String> {
    let session=branch.scope.clone();let session=session.as_str();
    if let Some(error)=&branch.error {return Err(error.clone());}
    // Fix the physical boundary before waiting. An append never silently extends an export.
    for source in &mut branch.sources {source.segment.end_byte=Some(source.segment.end_byte.unwrap_or(source.size));}
    let reader=branch.store.subscribe(session,&branch.sources,None)?;
    let mut observed=std::collections::HashSet::new();
    let mut refreshed=std::time::Instant::now();
    let result=(|| {
        loop {
            if cancel.load(Ordering::Relaxed) {return Err("read_canceled".into());}
            if refreshed.elapsed()>=std::time::Duration::from_secs(60) {
                branch.store.subscribe(session,&branch.sources,Some(&reader))?;refreshed=std::time::Instant::now();
            }
            let conn=branch.store.connect()?;let mut done=true;
            for source in &mut branch.sources {
                let (generation,water,status,error):(String,u64,String,Option<String>)=conn.query_row(
                    "SELECT generation,indexed_bytes,status,error FROM sources WHERE id=?1",[&source.id],
                    |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).map_err(sql)?;
                if generation!=source.generation {return Err("source_changed: history generation changed while preparing the export".into());}
                if status=="error" {return Err(error.unwrap_or_else(||"index_incomplete: conversation contains unreadable records".into()));}
                if water<source.segment.end_byte.unwrap() || source.kiro_pair.is_some() && status!="complete" {
                    done=false;
                    let key=format!("{}:{}",source.id,source.generation);
                    if branch.store.jobs.lock().unwrap().contains_key(&key) {observed.insert(key);}
                    else {
                        if observed.contains(&key) {
                            // A durable watermark excludes the unfinished tail and stops at a native
                            // ordinal cutoff. Neither is a malformed middle-record gap.
                            if source.segment.end_ordinal.is_none() {
                                return Err("index_incomplete: conversation ends with an unfinished record; retry after it has been saved".into());
                            }
                            source.segment.end_byte=Some(water);continue;
                        }
                        branch.store.schedule(branch.kind,source.clone());
                        if branch.store.jobs.lock().unwrap().contains_key(&key) {observed.insert(key);}
                    }
                } else if branch.store.jobs.lock().unwrap().contains_key(&format!("{}:{}",source.id,source.generation)) {
                    // The watermark reaches the end before the scan selects the active branch.
                    done=false;
                }
            }
            if done {return Ok(branch.clone());}
            std::thread::park_timeout(std::time::Duration::from_millis(100));
        }
    })();
    branch.store.cancel_reader(session,&reader)?;result
}

#[derive(Clone)]
pub enum Piece {Literal(String),Field(RegisteredSource,String),NativeUser(RegisteredSource,String),VerifiedLiteral(RegisteredSource,Vec<String>,String)}
#[derive(Clone)]
pub struct Text {pub pieces:Vec<Piece>,pub quote_json:bool,pub separator:&'static str}
impl Text {
    pub(crate) fn literal(value:String) -> Self {Self {pieces:vec![Piece::Literal(value)],quote_json:false,separator:"\n"}}

    pub fn walk(&self,branch:&Branch,cancel:&AtomicBool,sink:&mut impl FnMut(&str)->Result<(),String>) -> Result<(),String> {
        if self.quote_json {sink("\"")?;}
        for (index,piece) in self.pieces.iter().enumerate() {
            if index>0 {sink(self.separator)?;}
            let mut emit=|text:&str| {
                if cancel.load(Ordering::Relaxed) {return Err("read_canceled".into());}
                if self.quote_json {let encoded=serde_json::to_string(text).map_err(|e|e.to_string())?;sink(&encoded[1..encoded.len()-1])} else {sink(text)}
            };
            match piece {
                Piece::Literal(text)=>emit(text)?,
                Piece::NativeUser(source,message)=>super::native_database::walk_user(branch,source,message,cancel,&mut emit)?,
                Piece::VerifiedLiteral(source,references,text)=> {
                    let conn=branch.store.connect()?;
                    for reference in references {let mut sequence=0;loop {
                        if cancel.load(Ordering::Relaxed) {return Err("read_canceled".into());}
                        let (_,_,end,size)=api::read_piece(&conn,source,reference,sequence,0)?;
                        if end>=size {break;}sequence+=1;
                    }}emit(text)?;
                },
                Piece::Field(source,reference)=> {
                    let conn=branch.store.connect()?;let mut sequence=0;
                    loop {
                        let (text,_,end,size)=api::read_piece(&conn,source,reference,sequence,0)?;emit(&text)?;
                        if end>=size {break;}sequence+=1;
                    }
                }
            }
        }
        if self.quote_json {sink("\"")?;}Ok(())
    }

    pub fn write(&self,branch:&Branch,cancel:&AtomicBool,writer:&mut impl Write,trim:bool) -> Result<(),String> {
        let mut begin=0u64;let mut end=u64::MAX;
        if trim {
            let mut offset=0u64;let mut first=None;let mut last=0;
            self.walk(branch,cancel,&mut |text| {
                for (at,ch) in text.char_indices() {if !ch.is_whitespace() {first.get_or_insert(offset+at as u64);last=offset+(at+ch.len_utf8()) as u64;}}
                offset+=text.len() as u64;Ok(())
            })?;
            begin=first.unwrap_or(0);end=last;
        }
        self.write_range(branch,cancel,writer,begin,end)
    }

    pub fn write_range(&self,branch:&Branch,cancel:&AtomicBool,writer:&mut impl Write,begin:u64,end:u64) -> Result<(),String> {
        let mut offset=0u64;
        self.walk(branch,cancel,&mut |text| {
            let from=begin.saturating_sub(offset).min(text.len() as u64) as usize;
            let to=end.saturating_sub(offset).min(text.len() as u64) as usize;
            if to>from {writer.write_all(&text.as_bytes()[from..to]).map_err(|e|e.to_string())?;}
            offset+=text.len() as u64;Ok(())
        })
    }
}

pub struct StreamEvent {pub event:Event,pub body:Text,pub(crate) row_id:Option<String>}
pub(crate) struct IndexedRecord {pub source:RegisteredSource,pub record:Record,pub references:std::collections::HashMap<String,String>}
pub struct Cursor {pub branch:Branch,pub(crate) conn:rusqlite::Connection,source:usize,after:Option<u64>,pending:VecDeque<StreamEvent>}
impl Cursor {
    pub fn new(branch:Branch,cancel:&AtomicBool) -> Result<Self,String> {
        let branch=super::reconciliation::native_context(&branch,cancel)?;
        let conn=branch.store.connect()?;
        for source in &branch.sources {
            let current:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM sources WHERE id=?1 AND generation=?2)",params![source.id,source.generation],|r|r.get(0)).map_err(sql)?;
            if !current {return Err("source_changed: original history generation is no longer current".into());}
        }
        conn.execute_batch("PRAGMA temp_store=FILE; BEGIN;
            CREATE TEMP TABLE selected_records(source_id TEXT,generation TEXT,start INTEGER,PRIMARY KEY(source_id,generation,start));").map_err(sql)?;
        for source in &branch.sources {
            if matches!(branch.kind,SessionKind::Claude|SessionKind::Pi|SessionKind::Omp)
                && !(branch.kind==SessionKind::Claude && branch.scope.starts_with("global-search:")) {
                conn.execute("WITH RECURSIVE leaf(id) AS (SELECT CASE WHEN event_type='last-prompt' AND json_extract(projection,'$.rewound')=1
                    THEN json_extract(projection,'$.leafUuid') ELSE native_id END FROM records WHERE source_id=?1 AND generation=?2 AND end<=?3
                    AND (native_id IS NOT NULL OR (event_type='last-prompt' AND json_extract(projection,'$.rewound')=1)) ORDER BY start DESC LIMIT 1),
                    chain(id) AS (SELECT id FROM leaf UNION SELECT r.parent_id FROM chain c CROSS JOIN records r ON r.native_id=c.id
                        WHERE r.source_id=?1 AND r.generation=?2 AND r.end<=?3 AND r.parent_id IS NOT NULL)
                    INSERT INTO selected_records SELECT source_id,generation,start FROM records WHERE source_id=?1 AND generation=?2 AND end<=?3
                        AND (native_id IS NULL OR native_id IN (SELECT id FROM chain))",params![source.id,source.generation,source.segment.end_byte.unwrap_or(source.size)]).map_err(sql)?;
            } else {
                conn.execute("INSERT INTO selected_records SELECT source_id,generation,start FROM records WHERE source_id=?1 AND generation=?2
                    AND end<=?3 AND active=1",params![source.id,source.generation,source.segment.end_byte.unwrap_or(source.size)]).map_err(sql)?;
            }
        }
        Ok(Self {branch,conn,source:0,after:None,pending:VecDeque::new()})
    }

    pub fn rewind(&mut self) {self.source=0;self.after=None;self.pending.clear();}
    /// Continue from the record of `source` that starts at `start`.
    pub(crate) fn seek(&mut self,source:usize,start:u64) {self.source=source;self.after=start.checked_sub(1);self.pending.clear();}

    pub(crate) fn next_record(&mut self,cancel:&AtomicBool) -> Result<Option<IndexedRecord>,String> {
        loop {
            if cancel.load(Ordering::Relaxed) {return Err("read_canceled".into());}
            let Some(source)=self.branch.sources.get(self.source) else {return Ok(None);};
            let conn=&self.conn;
            // Seek the selection itself: an IN subquery is rebuilt as a temporary table on every call,
            // which made reading a conversation quadratic in its record count.
            let record:Option<(u64,u64,String,Option<String>)>=conn.query_row("SELECT r.start,r.end,r.projection,r.error FROM selected_records s
                CROSS JOIN records r ON r.source_id=s.source_id AND r.generation=s.generation AND r.start=s.start
                WHERE s.source_id=?1 AND s.generation=?2 AND s.start>?3
                AND r.end<=?4 AND (?5 IS NULL OR r.ordinal IS NULL OR r.ordinal<?5) ORDER BY s.start LIMIT 1",
                params![source.id,source.generation,self.after.map_or(-1,|start|start as i64),source.segment.end_byte.unwrap_or(source.size),source.segment.end_ordinal],
                |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional().map_err(sql)?;
            let Some((start,end,raw,error))=record else {self.source+=1;self.after=None;continue;};
            if let Some(error)=error {return Err(format!("index_incomplete: {error}"));}self.after=Some(start);
            let value:Value=serde_json::from_str(&raw).map_err(|e|e.to_string())?;
            let mut fields=Vec::new();let mut references=std::collections::HashMap::new();
            let mut statement=conn.prepare("SELECT ref,path,raw_start,raw_end,decoded_bytes,encoding FROM fields
                WHERE source_id=?1 AND generation=?2 AND record_start=?3 ORDER BY raw_start,path").map_err(sql)?;
            let found=statement.query_map(params![source.id,source.generation,start],|r|Ok((r.get::<_,String>(0)?,Field {
                path:r.get(1)?,start:r.get(2)?,end:r.get(3)?,decoded_bytes:r.get(4)?,encoding:r.get(5)?,truncated:false,hash:None}))).map_err(sql)?;
            for found in found {
                let (reference,mut field)=found.map_err(sql)?;
                field.truncated=field.decoded_bytes>super::limits::PREVIEW_BYTES as u64
                    || field.encoding=="string" && value.pointer(&field.path).and_then(Value::as_str).is_none_or(|text|text.len() as u64!=field.decoded_bytes);
                references.insert(field.path.clone(),reference);fields.push(field);
            }
            let record=Record {start,end,value,fields,error:None,complete:true};
            // Semantic discriminators can be after a giant ignored payload. Verify
            // their own original ranges rather than trusting cached classification.
            for field in record.fields.iter().filter(|field|super::projection::identity_path(&field.path)) {
                if field.decoded_bytes>super::limits::MAX_KEY_BYTES as u64 {return Err("unsupported_record: original history identity exceeds its metadata budget".into());}
                let reference=references.get(&field.path).ok_or("index_incomplete: original history identity is unavailable")?;
                let mut proof=api::DecodedFieldReader::new(conn,source,reference,0,None,0)?;let mut bytes=[0;4096];
                loop {if cancel.load(Ordering::Relaxed) {return Err("read_canceled".into());}
                    if std::io::Read::read(&mut proof,&mut bytes).map_err(|e|e.to_string())?==0 {break;}}
            }
            return Ok(Some(IndexedRecord {source:source.clone(),record,references}));
        }
    }

    pub fn next(&mut self,cancel:&AtomicBool) -> Result<Option<StreamEvent>,String> {
        loop {
            if cancel.load(Ordering::Relaxed) {return Err("read_canceled".into());}
            if let Some(event)=self.pending.pop_front() {return Ok(Some(event));}
            let Some(IndexedRecord {source,record,references})=self.next_record(cancel)? else {return Ok(None);};
            let raw=record.value.to_string();let conn=&self.conn;
            if self.branch.kind==SessionKind::Opencode && record.value["info"]["role"]=="user" {
                if record.value["type"]!="opencode_message" {continue;}
                let message=record.value["info"]["id"].as_str().ok_or("unsupported_record: native user identity is unavailable")?;
                if let Some(text)=super::native_database::user_preview(conn,&source,message)? {
                    let at=record.value.pointer("/info/time/created").and_then(Value::as_i64).and_then(crate::agent::chat::opencode_timeline::iso_from_ms);
                    let row_id=Some(format!("native:{}",super::store::digest(format!("opencode:{}:u:{message}",source.segment.native_id).as_bytes())));
                    return Ok(Some(StreamEvent {event:Event::User {text,ts:at},body:Text {pieces:vec![Piece::NativeUser(source,message.into())],quote_json:false,separator:"\n"},row_id}));
                }continue;
            }
            let events=match self.branch.kind {
                SessionKind::Codex=>export::codex_events(&raw),SessionKind::Claude=>export::claude_events(&raw),
                SessionKind::Pi|SessionKind::Omp=>export::pi_events(&raw),
                SessionKind::Kiro=>if record.value["type"]=="kiro_message" {
                    let text=record.value["text"].as_str().unwrap_or_default().into();let ts=record.value["timestamp"].as_str().map(str::to_owned);
                    vec![if record.value["role"]=="user" {Event::User {text,ts}} else {Event::AssistantText {text,ts}}]
                } else if record.value["type"]=="kiro_pair" {
                    vec![Event::User {text:record.value["prompt"].as_str().unwrap_or_default().into(),ts:record.value["timestamp"].as_str().map(str::to_owned)},
                        Event::AssistantText {text:record.value["answer"].as_str().unwrap_or_default().into(),ts:None}]
                } else {Vec::new()},
                SessionKind::Opencode=> {
                    let message=crate::agent::opencode_store::OpencodeMessage {info:record.value["info"].clone(),
                        parts:if record.value["type"]=="opencode_part" {vec![record.value["part"].clone()]} else {vec![]}};
                    crate::agent::chat::opencode_timeline::events(&[message])
                },
                other=>return Err(format!("Streamed event adapter is unavailable for {} sessions",other.as_str())),
            };
            for (local,mut event) in events.into_iter().enumerate() {
                let (role,call,fallback,quote)=match &event {
                    Event::User {text,..}|Event::AssistantText {text,..}|Event::Command {text,..}=>("body",None,text.clone(),false),
                    Event::Thinking {text,..}=>("reasoning",None,text.clone(),false),
                    Event::Wake {summary,..}=>("body",None,summary.clone(),false),
                    Event::ToolUse {id,input,..}=>("toolInput",id.as_deref(),serde_json::to_string_pretty(input).map_err(|e|e.to_string())?,input.is_string()),
                    Event::ToolResult {id,text,..}=>("toolOutput",id.as_deref(),text.clone(),false),
                    Event::Shell {output,..}=>("toolOutput",None,output.clone(),false),
                };
                let paths=super::adapter::body_paths(&record,role,call,local);
                let roots:Vec<String>=paths.iter().filter(|path|!paths.iter().any(|parent|*path!=parent && path.starts_with(&(parent.clone()+"/"))))
                    .cloned().collect();
                let long=record.fields.iter().any(|field|roots.contains(&field.path) && field.truncated);
                let mut body=Text {pieces:roots.iter().filter_map(|path|references.get(path).map(|reference|Piece::Field(source.clone(),reference.clone()))).collect(),quote_json:quote,separator:if self.branch.kind==SessionKind::Codex && role=="reasoning" {"\n\n"} else {"\n"}};
                if record.value["type"]=="kiro_message" {body.separator="";}
                if body.pieces.is_empty() {body=Text::literal(fallback.clone());}
                else if !long {
                    body=Text {pieces:vec![Piece::VerifiedLiteral(source.clone(),roots.iter().filter_map(|path|references.get(path).cloned()).collect(),fallback.clone())],quote_json:false,separator:"\n"};
                }
                // Local command echoes and shell wrappers are semantic transformations of short fields.
                // Never substitute an untransformed original for one of those displayed values.
                if matches!(event,Event::Command {..}|Event::Shell {..}) {
                    if long {return Err("unsupported_record: this transformed shell or command body requires a streaming adapter".into());}
                    body=Text::literal(fallback.clone());
                }
                if let Event::ToolResult {id:Some(id),name,..}=&mut event {
                    for candidate in self.branch.sources[..=self.source].iter().rev() {
                        *name=conn.query_row("SELECT json_extract(p.summary,'$.name') FROM row_parts p JOIN rows r
                            ON r.source_id=p.source_id AND r.generation=p.generation AND r.id=p.row_id
                            JOIN selected_records selected ON selected.source_id=p.source_id AND selected.generation=p.generation AND selected.start=p.record_start
                            WHERE r.source_id=?1 AND r.generation=?2 AND r.call_id=?3 AND json_extract(p.summary,'$.name')!='tool'
                            AND (?4 IS NULL OR p.ordinal IS NULL OR p.ordinal<?4) ORDER BY p.record_start LIMIT 1",
                            params![candidate.id,candidate.generation,id.as_str(),candidate.segment.end_ordinal],|r|r.get(0)).optional().map_err(sql)?;
                        if name.is_some() {break;}
                    }
                }
                let row_id=if self.branch.kind==SessionKind::Kiro {
                    record.value["id"].as_str().map(|id|format!("native:{}",super::store::digest(if record.value["type"]=="kiro_message" {
                        format!("kiro-file:{}:{id}",source.segment.native_id)
                    } else {format!("kiro:{}:{id}:{}",source.segment.native_id,if local==0 {"user"} else {"assistant"})}.as_bytes())))
                } else if self.branch.kind==SessionKind::Opencode {
                    record.value.pointer("/part/id").and_then(Value::as_str).map(|id|format!("native:{}",super::store::digest(format!("opencode:{}:{id}",source.segment.native_id).as_bytes())))
                } else {None};
                self.pending.push_back(StreamEvent {event,body,row_id});
            }
        }
    }
}

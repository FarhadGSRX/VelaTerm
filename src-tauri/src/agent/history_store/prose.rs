//! Fixed-snapshot prose messages. Range selection precedes reading any original message body.

use std::io::Write;
use std::sync::atomic::{AtomicBool,Ordering};
use rusqlite::{params,OptionalExtension};
use serde_json::{json,Value};
use crate::{agent::transcript,models::SessionKind};
use super::{api::Branch,event_stream::{Cursor,Piece,Text},store::sql};

pub struct Prose {pub(super) cursor:Cursor,pub total:u64}
/// Where a message begins: its first record and the piece within that record. `identity` names the
/// recording (its file identity and first bytes) and `before` counts the selected records ahead of the
/// message, so a later read can tell that nothing before it changed, even after the index was rebuilt.
#[derive(Clone,Debug,PartialEq)]
pub struct Start {pub source:String,pub record:u64,pub piece:u64,pub number:u64,pub identity:String,pub before:u64}
/// Grok renumbers messages after reading them all, and database providers read through events.
fn resumable(kind:SessionKind) -> bool {matches!(kind,SessionKind::Claude|SessionKind::Codex|SessionKind::Pi|SessionKind::Omp|SessionKind::Antigravity)}
fn identity(cursor:&Cursor,source:usize) -> Result<String,String> {
    let owner=&cursor.branch.sources[source];
    cursor.conn.query_row("SELECT identity||':'||hex(head_hash) FROM sources WHERE id=?1 AND generation=?2",params![owner.id,owner.generation],|r|r.get(0)).map_err(sql)
}
/// Selected records ahead of `record` of `source`, counting every earlier source in full.
fn before(cursor:&Cursor,source:usize,record:u64) -> Result<u64,String> {
    let mut count=0u64;
    for (index,owner) in cursor.branch.sources.iter().enumerate().take(source+1) {
        let limit=if index==source {record as i64} else {i64::MAX};
        count+=cursor.conn.query_row("SELECT count(*) FROM selected_records WHERE source_id=?1 AND generation=?2 AND start<?3",
            params![owner.id,owner.generation,limit],|r|r.get::<_,u64>(0)).map_err(sql)?;
    }
    Ok(count)
}
fn selected(cursor:&Cursor,source:usize,record:u64) -> Result<bool,String> {
    let owner=&cursor.branch.sources[source];
    cursor.conn.query_row("SELECT EXISTS(SELECT 1 FROM selected_records WHERE source_id=?1 AND generation=?2 AND start=?3)",
        params![owner.id,owner.generation,record as i64],|r|r.get(0)).map_err(sql)
}
pub fn supported(kind:SessionKind) -> bool {matches!(kind,SessionKind::Claude|SessionKind::Codex|SessionKind::Pi|SessionKind::Omp|SessionKind::Opencode|SessionKind::Kiro|SessionKind::Grok|SessionKind::Antigravity)}
impl Prose {
    pub fn new(branch:Branch,cancel:&AtomicBool) -> Result<Self,String> {
        let _work=super::work::Work::acquire(cancel)?;
        Self::build(Cursor::new(branch,cancel)?,cancel,None)
    }

    /// Messages from `start` on of a conversation that only grew since `start` was read, beginning at
    /// the record (and piece within it) where that message began. Earlier messages are not read.
    /// None when the recording or the selection before that message is no longer the same.
    pub fn resume(branch:Branch,cancel:&AtomicBool,start:&Start) -> Result<Option<Self>,String> {
        if !resumable(branch.kind) {return Ok(None);}
        let _work=super::work::Work::acquire(cancel)?;
        let cursor=Cursor::new(branch,cancel)?;
        let Some(source)=cursor.branch.sources.iter().position(|source|source.id==start.source) else {return Ok(None);};
        if identity(&cursor,source)?!=start.identity || before(&cursor,source,start.record)?!=start.before {return Ok(None);}
        // Pieces of the message before it share the first record; that record must still be selected.
        if start.piece>0 && !selected(&cursor,source,start.record)? {return Ok(None);}
        Self::build(cursor,cancel,Some((source,start))).map(Some)
    }

    /// Where message `number` begins, for a later `resume`.
    pub fn start(&self,number:u64) -> Result<Option<Start>,String> {
        if !resumable(self.cursor.branch.kind) {return Ok(None);}
        let found:Option<(usize,u64,u64)>=self.cursor.conn.query_row("SELECT source,record,piece FROM prose_messages WHERE number=?1",[number],
            |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(sql)?;
        let Some((source,record,piece))=found else {return Ok(None);};
        Ok(Some(Start {source:self.cursor.branch.sources[source].id.clone(),record,piece,number,
            identity:identity(&self.cursor,source)?,before:before(&self.cursor,source,record)?}))
    }

    fn build(mut cursor:Cursor,cancel:&AtomicBool,resume:Option<(usize,&Start)>) -> Result<Self,String> {
        cursor.conn.execute_batch("CREATE TEMP TABLE prose_messages(number INTEGER PRIMARY KEY,role TEXT,timestamp TEXT,row_id TEXT,source INTEGER,record INTEGER,piece INTEGER);
            CREATE TEMP TABLE prose_fragments(sequence INTEGER PRIMARY KEY,message INTEGER,descriptor TEXT);
            CREATE INDEX prose_fragment_message ON prose_fragments(message,sequence);
            CREATE TEMP TABLE prose_tools(sequence INTEGER PRIMARY KEY,message INTEGER,name TEXT);
            CREATE INDEX prose_tool_message ON prose_tools(message,sequence);").map_err(sql)?;
        if matches!(cursor.branch.kind,SessionKind::Opencode|SessionKind::Kiro) {return Self::native(cursor,cancel);}
        let mut total=0;let mut fragments=0;let mut tools=0;let mut role="";let mut workflow=false;let mut last_tool=None;
        // Pieces before the resumed message in its first record belong to the message before it.
        let mut skip=None;
        if let Some((source,start))=resume {cursor.seek(source,start.record);total=start.number;skip=Some((start.record,start.piece));}
        while let Some(indexed)=cursor.next_record(cancel)? {
            let record=&indexed.record;let raw=record.value.to_string();
            let pieces=match cursor.branch.kind {
                SessionKind::Codex=>transcript::parse_codex_line(&raw).into_iter().collect::<Vec<_>>(),
                SessionKind::Claude=>transcript::parse_claude_line(&raw).into_iter().collect::<Vec<_>>(),
                SessionKind::Pi|SessionKind::Omp=>transcript::pi_pieces(&raw),
                SessionKind::Grok=>transcript::grok_pieces(&record.value),
                SessionKind::Antigravity=>{
                    let mut value=record.value.clone();
                    if value["type"]=="USER_INPUT" {
                        let visible:Option<String>=cursor.conn.query_row("SELECT json_extract(summary,'$.text') FROM row_parts WHERE source_id=?1 AND generation=?2
                            AND record_start=?3 AND json_extract(summary,'$.kind')='user' LIMIT 1",params![indexed.source.id,indexed.source.generation,record.start],|r|r.get(0)).optional().map_err(sql)?;
                        value["content"]=json!(visible.unwrap_or_default());
                    }
                    transcript::antigravity_pieces(&value.to_string())
                },
                _=>return Err("unsupported_record: streamed prose is unavailable for this provider".into()),
            };
            let mut assistant_text=0;
            // Text from a record the conversation view leaves out (Codex's injected AGENTS.md
            // instructions, for one) has no row through which its fields could be opened.
            let shown=pieces.is_empty() || cursor.conn.query_row("SELECT EXISTS(SELECT 1 FROM row_parts WHERE source_id=?1 AND generation=?2 AND record_start=?3)",
                params![indexed.source.id,indexed.source.generation,record.start],|r|r.get::<_,bool>(0)).map_err(sql)?;
            let first=skip.take().filter(|(at,_)|*at==record.start).map_or(0,|(_,piece)|piece);
            for (index,piece) in pieces.into_iter().enumerate() {
                if (index as u64)<first {continue;}
                if !shown && !piece.text.is_empty() {continue;}
                let boundary=cursor.branch.kind!=SessionKind::Grok && piece.role=="user" && (workflow || piece.text.starts_with("[VelaTerm message "));
                if role!=piece.role || boundary {
                    total+=1;role=piece.role;workflow=piece.text.starts_with("[VelaTerm message ");last_tool=None;
                    let row_id:Option<String>=cursor.conn.query_row("SELECT row_id FROM row_parts WHERE source_id=?1 AND generation=?2 AND record_start=?3
                        ORDER BY CASE WHEN json_extract(summary,'$.kind')=?4 THEN 0 ELSE 1 END,row_id LIMIT 1",
                        params![indexed.source.id,indexed.source.generation,record.start,role],|r|r.get(0)).optional().map_err(sql)?;
                    let source=cursor.branch.sources.iter().position(|source|source.id==indexed.source.id && source.generation==indexed.source.generation)
                        .ok_or("source_changed: prose source is outside the fixed snapshot")?;
                    cursor.conn.execute("INSERT INTO prose_messages VALUES(?1,?2,?3,?4,?5,?6,?7)",params![total-1,role,piece.timestamp,row_id,source,record.start,index as u64]).map_err(sql)?;
                }
                if !piece.text.is_empty() {
                    let mut paths=super::adapter::body_paths(record,"body",None,0);
                    if cursor.branch.kind==SessionKind::Antigravity {
                        paths=vec![if role=="user" {"@visible/content"} else {"/content"}.into()];
                    }
                    // Claude and Pi can place several assistant blocks in one physical row. Select
                    // only communicated text, preserving their existing blank-line separators.
                    if role=="assistant" && matches!(cursor.branch.kind,SessionKind::Claude|SessionKind::Pi|SessionKind::Omp) {
                        paths=record.fields.iter().filter(|field|field.encoding=="string" && field.path.starts_with("/message/content/")
                            && field.path.ends_with("/text")).map(|field|field.path.clone()).collect();
                        if matches!(cursor.branch.kind,SessionKind::Pi|SessionKind::Omp) {paths=paths.into_iter().nth(assistant_text).into_iter().collect();assistant_text+=1;}
                    }
                    let long=record.fields.iter().any(|field|paths.contains(&field.path) && field.truncated)
                        || cursor.branch.kind==SessionKind::Antigravity && role=="user";
                    let source=cursor.branch.sources.iter().position(|source|source.id==indexed.source.id && source.generation==indexed.source.generation)
                        .ok_or("source_changed: prose source is outside the fixed snapshot")?;
                    let references=paths.iter().filter_map(|path|indexed.references.get(path)).collect::<Vec<_>>();
                    let proofs=record.fields.iter().filter(|field|super::projection::identity_path(&field.path)).filter_map(|field|indexed.references.get(&field.path)).collect::<Vec<_>>();
                    let descriptor=if long {
                        if paths.is_empty() {return Err("unsupported_record: original prose fields are unavailable".into());}
                        json!({"source":source,"references":references,
                            "separator":if cursor.branch.kind==SessionKind::Grok {""} else if role=="assistant" && cursor.branch.kind!=SessionKind::Codex {"\n\n"} else {"\n"},
                            "trim":!matches!(cursor.branch.kind,SessionKind::Grok|SessionKind::Antigravity),"join":if cursor.branch.kind==SessionKind::Grok {""} else {"\n\n"},"proofs":proofs})
                    } else {json!({"source":source,"references":references,"literal":piece.text,"proofs":proofs,"join":if cursor.branch.kind==SessionKind::Grok {""} else {"\n\n"}})};
                    cursor.conn.execute("INSERT INTO prose_fragments VALUES(?1,?2,?3)",params![fragments,total-1,descriptor.to_string()]).map_err(sql)?;fragments+=1;
                }
                for tool in piece.tools {
                    if last_tool.as_ref()!=Some(&tool) {
                        cursor.conn.execute("INSERT INTO prose_tools VALUES(?1,?2,?3)",params![tools,total-1,tool]).map_err(sql)?;tools+=1;
                        last_tool=Some(tool);
                    }
                }
            }
        }
        let mut prose=Self {cursor,total};
        if prose.cursor.branch.kind==SessionKind::Grok {prose.compact_grok(cancel)?;}
        Ok(prose)
    }

    fn compact_grok(&mut self,cancel:&AtomicBool) -> Result<(),String> {
        struct Meaningful(bool);
        impl Write for Meaningful {
            fn write(&mut self,bytes:&[u8]) -> std::io::Result<usize> {
                self.0|=std::str::from_utf8(bytes).map_err(std::io::Error::other)?.chars().any(|ch|!ch.is_whitespace());Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {Ok(())}
        }
        self.cursor.conn.execute_batch("CREATE TEMP TABLE grok_numbers(old INTEGER PRIMARY KEY,new INTEGER);").map_err(sql)?;let mut total=0;
        for number in 0..self.total {
            let mut text=Meaningful(false);self.body(number,cancel,&mut text)?;
            let tools:bool=self.cursor.conn.query_row("SELECT EXISTS(SELECT 1 FROM prose_tools WHERE message=?1)",[number],|r|r.get(0)).map_err(sql)?;
            if text.0 || tools {self.cursor.conn.execute("INSERT INTO grok_numbers VALUES(?1,?2)",params![number,total]).map_err(sql)?;total+=1;}
        }
        for table in ["prose_fragments","prose_tools"] {
            self.cursor.conn.execute(&format!("DELETE FROM {table} WHERE message NOT IN (SELECT old FROM grok_numbers)"),[]).map_err(sql)?;
            self.cursor.conn.execute(&format!("UPDATE {table} SET message=(SELECT new FROM grok_numbers WHERE old=message)"),[]).map_err(sql)?;
        }
        self.cursor.conn.execute_batch("DELETE FROM prose_messages WHERE number NOT IN (SELECT old FROM grok_numbers);
            UPDATE prose_messages SET number=-(SELECT new FROM grok_numbers WHERE old=number)-1;UPDATE prose_messages SET number=-number-1;
            DROP TABLE grok_numbers;").map_err(sql)?;self.total=total;Ok(())
    }

    fn native(mut cursor:Cursor,cancel:&AtomicBool) -> Result<Self,String> {
        use crate::agent::export::Event;
        let mut total=0;let mut fragments=0;let mut tools=0;let mut role="";let mut workflow=false;let mut last_tool=None;
        while let Some(item)=cursor.next(cancel)? {
            let (next,text,timestamp,tool)=match &item.event {
                Event::User {text,ts}=>("user",Some(text),ts,None),
                Event::AssistantText {text,ts}=>("assistant",Some(text),ts,None),
                Event::ToolUse {name,ts,..}=>("assistant",None,ts,Some(name)),
                _=>continue,
            };
            let workflow_body=text.is_some_and(|text|text.starts_with("[VelaTerm message "));
            if role!=next || cursor.branch.kind==SessionKind::Kiro && text.is_some() || next=="user" && (workflow || workflow_body) {
                total+=1;role=next;workflow=workflow_body;last_tool=None;
                cursor.conn.execute("INSERT INTO prose_messages(number,role,timestamp,row_id) VALUES(?1,?2,?3,?4)",params![total-1,role,timestamp,item.row_id]).map_err(sql)?;
            }
            if text.is_some_and(|text|!text.is_empty()) || cursor.branch.kind==SessionKind::Kiro && text.is_some()
                || item.body.pieces.iter().any(|piece|matches!(piece,Piece::NativeUser(..))) {
                let mut source=0;let mut references=Vec::new();let mut literal=None;let mut native_user=None;
                for piece in &item.body.pieces {
                    let owner=match piece {Piece::Field(owner,_)|Piece::NativeUser(owner,_)|Piece::VerifiedLiteral(owner,..)=>Some(owner),_=>None};
                    if let Some(owner)=owner {source=cursor.branch.sources.iter().position(|candidate|candidate.id==owner.id && candidate.generation==owner.generation)
                        .ok_or("source_changed: native prose source is unavailable")?;}
                    match piece {
                        Piece::Field(_,reference)=>references.push(reference.clone()),
                        Piece::NativeUser(_,message)=>native_user=Some(message.clone()),
                        Piece::Literal(text)=>literal=Some(text.clone()),
                        Piece::VerifiedLiteral(_,fields,text)=>{references.extend(fields.iter().cloned());literal=Some(text.clone());},
                    }
                }
                let descriptor=json!({"source":source,"references":references,"literal":literal,"nativeUser":native_user,"trim":false,"separator":item.body.separator});
                cursor.conn.execute("INSERT INTO prose_fragments VALUES(?1,?2,?3)",params![fragments,total-1,descriptor.to_string()]).map_err(sql)?;fragments+=1;
            }
            if let Some(tool)=tool {if last_tool.as_ref()!=Some(tool) {
                cursor.conn.execute("INSERT INTO prose_tools VALUES(?1,?2,?3)",params![tools,total-1,tool]).map_err(sql)?;tools+=1;last_tool=Some(tool.clone());
            }}
        }Ok(Self {cursor,total})
    }

    pub fn row_id(&self,number:u64) -> Result<Option<String>,String> {
        self.cursor.conn.query_row("SELECT row_id FROM prose_messages WHERE number=?1",[number],|r|r.get(0)).optional().map_err(sql).map(Option::flatten)
    }

    pub fn metadata(&self,number:u64) -> Result<Option<Value>,String> {
        self.cursor.conn.query_row("SELECT role,timestamp,row_id FROM prose_messages WHERE number=?1",[number],|row| {
            Ok(json!({"messageIndex":number,"role":row.get::<_,String>(0)?,"timestamp":row.get::<_,Option<String>>(1)?,"rowId":row.get::<_,Option<String>>(2)?}))
        }).optional().map_err(sql)
    }

    pub fn memory_source(&self,max_chars:usize,cancel:&AtomicBool) -> Result<Option<String>,String> {
        let mut writer=Characters {text:String::new(),chars:0,limit:max_chars};let mut meaningful=false;
        for number in 0..self.total {
            let meta=self.metadata(number)?.ok_or("source_changed: message metadata changed")?;
            if number>0 {writer.write_all(b"\n").map_err(|e|e.to_string())?;}
            write!(writer,"## Message {} · {} · {}\n\n",number+1,meta["role"].as_str().unwrap_or_default(),meta["timestamp"].as_str().unwrap_or_default()).map_err(|e|e.to_string())?;
            let before=writer.text.len();self.body(number,cancel,&mut writer)?;
            meaningful|=!writer.text[before..].trim().is_empty();writer.write_all(b"\n").map_err(|e|e.to_string())?;
        }
        Ok(meaningful.then_some(writer.text))
    }

    pub(crate) fn body(&self,number:u64,cancel:&AtomicBool,writer:&mut impl Write) -> Result<(),String> {
        let _work=super::work::Work::acquire(cancel)?;
        let mut statement=self.cursor.conn.prepare("SELECT descriptor FROM prose_fragments WHERE message=?1 ORDER BY sequence").map_err(sql)?;
        let fragments=statement.query_map([number],|row|row.get::<_,String>(0)).map_err(sql)?;
        let mut first=true;
        for fragment in fragments {
            if cancel.load(Ordering::Relaxed) {return Err("read_canceled".into());}
            let value:Value=serde_json::from_str(&fragment.map_err(sql)?).map_err(|e|e.to_string())?;
            if !first {writer.write_all(if value["join"]=="" {b""} else {b"\n\n"}).map_err(|e|e.to_string())?;}first=false;
            let source=self.cursor.branch.sources.get(value["source"].as_u64().ok_or("Invalid prose source")? as usize).ok_or("Invalid prose source")?;
            if let Some(proofs)=value["proofs"].as_array() {let conn=self.cursor.branch.store.connect()?;
                for proof in proofs {let reference=proof.as_str().ok_or("Invalid prose proof")?;
                    let mut proof=super::api::DecodedFieldReader::new(&conn,source,reference,0,None,0)?;let mut bytes=[0;4096];
                    loop {if cancel.load(Ordering::Relaxed) {return Err("read_canceled".into());}if std::io::Read::read(&mut proof,&mut bytes).map_err(|e|e.to_string())?==0 {break;}}
                }
            }
            if let Some(message)=value["nativeUser"].as_str() {
                super::native_database::walk_user(&self.cursor.branch,source,message,cancel,&mut |text|writer.write_all(text.as_bytes()).map_err(|e|e.to_string()))?;continue;
            }
            let pieces=value["references"].as_array().ok_or("Invalid prose fields")?.iter().map(|reference| {
                reference.as_str().map(|reference|Piece::Field(source.clone(),reference.into())).ok_or("Invalid prose field")
            }).collect::<Result<Vec<_>,_>>()?;
            let text=Text {pieces,quote_json:false,separator:if value["separator"]=="" {""} else if value["separator"]=="\n\n" {"\n\n"} else {"\n"}};
            if let Some(literal)=value["literal"].as_str() {
                // Short transformed text is cached, but its contributing bytes are still verified
                // against the actual source before it is supplied as original conversation evidence.
                text.walk(&self.cursor.branch,cancel,&mut |_|Ok(()))?;
                writer.write_all(literal.as_bytes()).map_err(|e|e.to_string())?;continue;
            }
            text.write(&self.cursor.branch,cancel,writer,value["trim"]==true)?;
        }
        Ok(())
    }

    pub fn write_messages(&self,start:u64,end:u64,cancel:&AtomicBool,writer:&mut impl Write) -> Result<(),String> {
        writer.write_all(b"[").map_err(|e|e.to_string())?;
        for number in start.min(self.total)..end.min(self.total).max(start.min(self.total)) {
            if cancel.load(Ordering::Relaxed) {return Err("read_canceled".into());}
            if number>start {writer.write_all(b",").map_err(|e|e.to_string())?;}
            let (role,timestamp):(String,Option<String>)=self.cursor.conn.query_row("SELECT role,timestamp FROM prose_messages WHERE number=?1",[number],|r|Ok((r.get(0)?,r.get(1)?))).map_err(sql)?;
            writer.write_all(b"{\"role\":").map_err(|e|e.to_string())?;serde_json::to_writer(&mut *writer,&role).map_err(|e|e.to_string())?;
            writer.write_all(b",\"timestamp\":").map_err(|e|e.to_string())?;serde_json::to_writer(&mut *writer,&timestamp).map_err(|e|e.to_string())?;
            writer.write_all(b",\"tools\":[").map_err(|e|e.to_string())?;
            let mut statement=self.cursor.conn.prepare("SELECT name FROM prose_tools WHERE message=?1 ORDER BY sequence").map_err(sql)?;
            let tools=statement.query_map([number],|r|r.get::<_,String>(0)).map_err(sql)?;
            for (index,tool) in tools.enumerate() {if index>0 {writer.write_all(b",").map_err(|e|e.to_string())?;}serde_json::to_writer(&mut *writer,&tool.map_err(sql)?).map_err(|e|e.to_string())?;}
            writer.write_all(b"],\"text\":\"").map_err(|e|e.to_string())?;
            self.body(number,cancel,&mut JsonString {writer})?;
            writer.write_all(b"\"}").map_err(|e|e.to_string())?;
        }
        writer.write_all(b"]").map_err(|e|e.to_string())?;Ok(())
    }

    pub fn write_text(&self,start:u64,end:u64,cancel:&AtomicBool,writer:&mut impl Write) -> Result<(),String> {
        for number in start.min(self.total)..end.min(self.total).max(start.min(self.total)) {
            let (role,timestamp):(String,Option<String>)=self.cursor.conn.query_row("SELECT role,timestamp FROM prose_messages WHERE number=?1",[number],|r|Ok((r.get(0)?,r.get(1)?))).map_err(sql)?;
            let stamp=timestamp.as_deref().map(crate::agent::cli_client::short_time).filter(|value|!value.is_empty()).map(|value|format!(" {value}")).unwrap_or_default();
            write!(writer,"\n[#{number} {role}{stamp}]").map_err(|e|e.to_string())?;
            let mut statement=self.cursor.conn.prepare("SELECT name FROM prose_tools WHERE message=?1 ORDER BY sequence").map_err(sql)?;
            let tools=statement.query_map([number],|r|r.get::<_,String>(0)).map_err(sql)?;let mut first=true;
            for tool in tools {writer.write_all(if first {b" ("} else {b", "}).map_err(|e|e.to_string())?;first=false;writer.write_all(tool.map_err(sql)?.as_bytes()).map_err(|e|e.to_string())?;}
            if !first {writer.write_all(b")").map_err(|e|e.to_string())?;}
            writer.write_all(b"\n").map_err(|e|e.to_string())?;self.body(number,cancel,writer)?;writer.write_all(b"\n").map_err(|e|e.to_string())?;
        }Ok(())
    }

    /// Preserve the existing model-context rendering without materializing all message bodies.
    pub fn write_model_context(&self,cancel:&AtomicBool,writer:&mut impl Write) -> Result<(),String> {
        for number in 0..self.total {
            let (role,timestamp):(String,Option<String>)=self.cursor.conn.query_row("SELECT role,timestamp FROM prose_messages WHERE number=?1",[number],|r|Ok((r.get(0)?,r.get(1)?))).map_err(sql)?;
            write!(writer,"\n[#{number} {role}]").map_err(|e|e.to_string())?;
            if let Some(timestamp)=timestamp.filter(|value|!value.is_empty()) {write!(writer," {timestamp}").map_err(|e|e.to_string())?;}
            let mut statement=self.cursor.conn.prepare("SELECT name FROM prose_tools WHERE message=?1 ORDER BY sequence").map_err(sql)?;
            let tools=statement.query_map([number],|r|r.get::<_,String>(0)).map_err(sql)?;let mut first=true;
            for tool in tools {writer.write_all(if first {b" (tools: "} else {b", "}).map_err(|e|e.to_string())?;first=false;writer.write_all(tool.map_err(sql)?.as_bytes()).map_err(|e|e.to_string())?;}
            if !first {writer.write_all(b")").map_err(|e|e.to_string())?;}
            writer.write_all(b"\n").map_err(|e|e.to_string())?;self.body(number,cancel,writer)?;writer.write_all(b"\n").map_err(|e|e.to_string())?;
        }Ok(())
    }

    pub fn collect_bounded(&self,start:u64,end:u64,max_bytes:usize,cancel:&AtomicBool) -> Result<Vec<transcript::TranscriptMessage>,String> {
        let mut writer=Budget {bytes:Vec::new(),limit:max_bytes};self.write_messages(start,end,cancel,&mut writer)?;
        serde_json::from_slice(&writer.bytes).map_err(|e|e.to_string())
    }
}

struct JsonString<'a,W:Write> {writer:&'a mut W}
impl<W:Write> Write for JsonString<'_,W> {
    fn write(&mut self,bytes:&[u8]) -> std::io::Result<usize> {
        let text=std::str::from_utf8(bytes).map_err(std::io::Error::other)?;
        let value=serde_json::to_string(text).map_err(std::io::Error::other)?;
        self.writer.write_all(value[1..value.len()-1].as_bytes())?;Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {self.writer.flush()}
}

struct Budget {bytes:Vec<u8>,limit:usize}
impl Write for Budget {
    fn write(&mut self,bytes:&[u8]) -> std::io::Result<usize> {
        if self.bytes.len().saturating_add(bytes.len())>self.limit {return Err(std::io::Error::other(super::compat::UPGRADE));}
        self.bytes.extend_from_slice(bytes);Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {Ok(())}
}

struct Characters {text:String,chars:usize,limit:usize}
impl Write for Characters {
    fn write(&mut self,bytes:&[u8]) -> std::io::Result<usize> {
        let text=std::str::from_utf8(bytes).map_err(std::io::Error::other)?;let count=text.chars().count();
        if count>self.limit.saturating_sub(self.chars) {return Err(std::io::Error::other("memory_source_too_large"));}
        self.text.push_str(text);self.chars+=count;Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {Ok(())}
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn grok_chunks_and_antigravity_visible_requests_preserve_original_prose_semantics() {
        let cancel=AtomicBool::new(false);let temp=super::super::tests::Temp::new();
        let records=[json!({"method":"session/update","timestamp":1,"params":{"update":{"sessionUpdate":"user_message_chunk","content":{"text":"  "}}}}),
            json!({"method":"session/update","timestamp":2,"params":{"update":{"sessionUpdate":"agent_message_chunk","content":{"text":"first"}}}}),
            json!({"method":"session/update","params":{"update":{"sessionUpdate":"agent_message_chunk","content":{"text":" second\n"}}}}),
            json!({"method":"session/update","params":{"update":{"sessionUpdate":"tool_call","_meta":{"x.ai/tool":{"name":"Read"}}}}}),
            json!({"method":"session/update","params":{"update":{"sessionUpdate":"user_message_chunk","content":{"text":"[VelaTerm message one]"}}}}),
            json!({"method":"session/update","params":{"update":{"sessionUpdate":"user_message_chunk","content":{"text":"[VelaTerm message two]"}}}})];
        let raw=records.iter().map(|record|record.to_string()+"\n").collect::<String>();let branch=super::super::tests::indexed_kind(&temp,&raw,SessionKind::Grok);
        let expected=transcript::read_at(SessionKind::Grok,&branch.sources[0].segment.path).unwrap();let prose=Prose::new(branch,&cancel).unwrap();
        assert_eq!(prose.collect_bounded(0,prose.total,512*1024,&cancel).unwrap(),expected);assert_eq!(prose.total,2);
        let temp=super::super::tests::Temp::new();let request="中文 original\n".repeat(5000);let answer=" \n".to_owned()+&"response\n".repeat(5000)+" \t";
        let raw=[json!({"type":"USER_INPUT","step_index":0,"content":format!("{}<USER_REQUEST> {request} </USER_REQUEST>private tail","private head ".repeat(1000))}),
            json!({"type":"PLANNER_RESPONSE","step_index":1,"content":answer,"thinking":"private reasoning","tool_calls":[{"name":"view_file","args":{}}]})]
            .iter().map(|record|record.to_string()+"\n").collect::<String>();
        let branch=super::super::tests::indexed_kind(&temp,&raw,SessionKind::Antigravity);let expected=transcript::read_at(SessionKind::Antigravity,&branch.sources[0].segment.path).unwrap();
        let prose=Prose::new(branch,&cancel).unwrap();assert_eq!(prose.collect_bounded(0,prose.total,512*1024,&cancel).unwrap(),expected);
    }
    #[test]
    fn exhausted_previews_keep_late_text_and_injected_context_visibility() {
        for kind in [SessionKind::Claude,SessionKind::Codex,SessionKind::Pi] {
            let temp=super::super::tests::Temp::new();let noise=json!((0..20).map(|_|json!({"text":"noise".repeat(2000)})).collect::<Vec<_>>()).to_string();
            let body=" ".repeat(12_000)+"Late communicated original";
            let value=match kind {
                SessionKind::Claude=>json!({"type":"assistant","uuid":"one","message":{"content":[{"type":"text","text":body}]}}),
                SessionKind::Pi=>json!({"type":"message","id":"one","message":{"role":"assistant","content":[{"type":"text","text":body}]}}),
                _=>json!({"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":body}]}}),
            };
            let raw=format!("{{\"noise\":{noise},{}\n",&value.to_string()[1..]);
            let branch=super::super::tests::indexed_kind(&temp,&raw,kind);
            assert_eq!(branch.page(None,None,None).unwrap()["rows"].as_array().unwrap().iter().filter(|row|row["kind"]=="assistant").count(),1,"{kind:?}");
            let prose=Prose::new(branch,&AtomicBool::new(false)).unwrap();assert_eq!(prose.total,1,"{kind:?}");
            assert_eq!(prose.collect_bounded(0,1,64*1024,&AtomicBool::new(false)).unwrap()[0].text,"Late communicated original","{kind:?}");
            if kind!=SessionKind::Pi {
                let temp=super::super::tests::Temp::new();let mut value=value;
                let content=if kind==SessionKind::Claude {value["type"]=json!("user");&mut value["message"]["content"][0]} else {value["payload"]["role"]=json!("user");&mut value["payload"]["content"][0]};
                content["text"]=json!(" ".repeat(12_000)+if kind==SessionKind::Claude {"<system-reminder>injected</system-reminder>"} else {"<environment_context>injected</environment_context>"});
                let raw=format!("{{\"noise\":{noise},{}\n",&value.to_string()[1..]);let branch=super::super::tests::indexed_kind(&temp,&raw,kind);
                assert_eq!(Prose::new(branch,&AtomicBool::new(false)).unwrap().total,0,"{kind:?}");
            }
        }
    }
    #[test]
    fn codex_instructions_hidden_from_the_conversation_stay_out_of_prose() {
        let temp=super::super::tests::Temp::new();
        let message=|role:&str,text:&str| json!({"type":"response_item","payload":{"type":"message","role":role,
            "content":[{"type":if role=="user" {"input_text"} else {"output_text"},"text":text}]}}).to_string()+"\n";
        let raw=message("user","# AGENTS.md instructions\n\n<INSTRUCTIONS>rules</INSTRUCTIONS>")+&message("user","Where is the config?")+&message("assistant","In settings.");
        let branch=super::super::tests::indexed(&temp,&raw);
        assert_eq!(branch.page(None,None,None).unwrap()["rows"].as_array().unwrap().len(),2);
        let cancel=AtomicBool::new(false);let prose=Prose::new(branch,&cancel).unwrap();assert_eq!(prose.total,2);
        let texts=prose.collect_bounded(0,2,64*1024,&cancel).unwrap().into_iter().map(|message|message.text).collect::<Vec<_>>();
        assert_eq!(texts,["Where is the config?","In settings."]);
    }
    #[test]
    fn model_context_keeps_the_legacy_format_and_validates_small_original_bodies() {
        use std::io::{Seek,SeekFrom};
        let temp=super::super::tests::Temp::new();
        let raw=json!({"type":"response_item","timestamp":"2026-10-09T00:00:00Z","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"original question"}]}}).to_string()+"\n";
        let branch=super::super::tests::indexed(&temp,&raw);let path=branch.sources[0].segment.path.clone();let cancel=AtomicBool::new(false);
        let prose=Prose::new(branch,&cancel).unwrap();let mut context=Vec::new();prose.write_model_context(&cancel,&mut context).unwrap();
        assert_eq!(String::from_utf8(context).unwrap(),"\n[#0 user] 2026-10-09T00:00:00Z\noriginal question\n");
        let mut file=std::fs::OpenOptions::new().write(true).open(&path).unwrap();
        file.seek(SeekFrom::Start(raw.find("original question").unwrap() as u64)).unwrap();file.write_all(b"replaced question").unwrap();
        assert!(prose.write_model_context(&cancel,&mut Vec::new()).unwrap_err().starts_with("source_changed:"));
    }

    #[test]
    fn memory_material_preserves_the_existing_character_limit_and_full_original_text() {
        let temp=super::super::tests::Temp::new();let text="雪😀".repeat(100_000);
        let raw=json!({"type":"response_item","timestamp":"t1","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":text}]}}).to_string()+"\n";
        let branch=super::super::tests::indexed(&temp,&raw);let path=branch.sources[0].segment.path.clone();let cancel=AtomicBool::new(false);
        assert_eq!(super::super::compat::read_path(&path).unwrap_err(),super::super::compat::UPGRADE);
        let prose=Prose::new(branch,&cancel).unwrap();
        let expected=format!("## Message 1 · user · t1\n\n{text}\n");
        assert_eq!(prose.memory_source(expected.chars().count(),&cancel).unwrap().unwrap(),expected);
        assert_eq!(prose.memory_source(expected.chars().count()-1,&cancel).unwrap_err(),"memory_source_too_large");
    }
    #[test]
    fn prose_matches_existing_merge_and_selects_range_before_long_body_reads() {
        let temp=super::super::tests::Temp::new();let long="雪😀\n".repeat(20_000);let mut raw=String::new();
        for value in [json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"First"}]}}),
            json!({"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":long}]}}),
            json!({"type":"response_item","payload":{"type":"function_call","name":"Read","call_id":"one","arguments":"{}"}}),
            json!({"type":"response_item","payload":{"type":"function_call","name":"Read","call_id":"two","arguments":"{}"}}),
            json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"Last"}]}})] {raw.push_str(&value.to_string());raw.push('\n');}
        let branch=super::super::tests::indexed(&temp,&raw);let path=branch.sources[0].segment.path.clone();let cancel=AtomicBool::new(false);
        let prose=Prose::new(branch,&cancel).unwrap();assert_eq!(prose.total,3);
        let all=prose.collect_bounded(0,3,512*1024,&cancel).unwrap();assert_eq!(all,transcript::read_at(SessionKind::Codex,&path).unwrap());
        assert_eq!(all[1].text,long.trim_end());assert_eq!(all[1].tools,["Read"]);
        // Break an earlier original after metadata selection. Reading the last message must never
        // touch the unrelated giant body, while requesting that body detects the changed bytes.
        let original=std::fs::read_to_string(&path).unwrap();std::fs::write(&path,original.replace("雪","雲")).unwrap();
        assert_eq!(prose.collect_bounded(2,3,1024,&cancel).unwrap()[0].text,"Last");
        assert!(prose.collect_bounded(1,2,512*1024,&cancel).unwrap_err().contains("source_changed"));
    }
}

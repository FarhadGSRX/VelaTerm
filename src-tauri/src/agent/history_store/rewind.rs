//! Native rewind identities are selected from the current indexed branch, never from display text.

use rusqlite::{params,Connection,OptionalExtension};
use serde_json::{json,Value};
use crate::{agent::chat::history::RewindTarget,host::AppCtx,models::SessionKind};
use super::{api::{self,Branch},store::{RegisteredSource,sql}};

pub struct Selection {pub branch:Branch,pub row:String,pub target:RewindTarget,pub position:String}

impl Selection {
    pub fn prefill(&self) -> Result<String,String> {
        if !self.target.text.is_empty() {return Ok(self.target.text.clone());}
        let mut text=String::new();let mut cursor=None;let mut first=true;
        loop {
            let page=self.branch.manifest_page(&self.row,cursor.as_deref())?;
            for field in page["fields"].as_array().ok_or("index_incomplete: rewind body manifest is unavailable")? {
                if field["label"]!="body" {continue;}
                let reference=field["detailRef"].as_str().ok_or("index_incomplete: rewind body reference is unavailable")?;
                let size=field["sizeBytes"].as_u64().unwrap_or(0);
                let prefix=field["prefix"].as_str().unwrap_or_default();let suffix=field["suffix"].as_str().unwrap_or_default();
                if text.len() as u64+size+prefix.len() as u64+suffix.len() as u64+1>8*1024*1024 {
                    return Err("detail_requires_paging: rewind text exceeds the editable draft budget".into());
                }
                if !first {text.push('\n');}first=false;text.push_str(prefix);
                if let Some(literal)=field["literal"].as_str() {
                    self.branch.write_detail(reference,None,&mut std::io::sink(),&std::sync::atomic::AtomicBool::new(false))?;text.push_str(literal);
                } else {
                    let mut sequence=0;loop {let part=self.branch.detail(reference,None,sequence)?;
                        text.push_str(part["text"].as_str().ok_or("index_incomplete: rewind text chunk is unavailable")?);
                        if text.len()>8*1024*1024 {return Err("detail_requires_paging: rewind text exceeds the editable draft budget".into());}
                        if part["eof"]==true {break;}sequence+=1;
                    }
                }text.push_str(suffix);
            }
            cursor=page["nextCursor"].as_str().map(str::to_owned);if cursor.is_none() {break;}
        }
        if text.is_empty() {
            let page=self.branch.manifest(&self.row,0)?;
            text=page["row"]["text"].as_str().or_else(||page["row"]["command"].as_str()).unwrap_or_default().into();
        }
        if let Some(last)=&self.target.last_message_id {
            let conn=self.branch.store.connect()?;let mut found=false;
            for source in &self.branch.sources {
                let record:Option<(u64,u64,String)>=conn.query_row("SELECT start,end,projection FROM records WHERE source_id=?1 AND generation=?2 AND native_id=?3
                    AND active=1 AND (?4 IS NULL OR end<=?4) AND (?5 IS NULL OR ordinal IS NULL OR ordinal<?5)",
                    params![source.id,source.generation,last,source.segment.end_byte,source.segment.end_ordinal],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).optional().map_err(sql)?;
                if let Some((start,end,raw))=record {
                    let value:Value=serde_json::from_str(&raw).map_err(|_|"index_incomplete: native shell output projection is invalid")?;
                    let reference=super::shell_text::single(&conn,source,start,&value)?.ok_or("index_incomplete: native shell output has no original field")?;
                    let size:u64=conn.query_row("SELECT decoded_bytes FROM fields WHERE ref=?1",[&reference],|row|row.get(0)).map_err(sql)?;
                    if text.len() as u64+size+1>8*1024*1024 {return Err("detail_requires_paging: rewind text exceeds the editable draft budget".into());}
                    verify(&conn,source,start,end)?;text.push('\n');let mut sequence=0;
                    loop {let (part,_,end,size)=api::read_piece(&conn,source,&reference,sequence,0)?;text.push_str(&part);if end>=size {break;}sequence+=1;}
                    found=true;break;
                }
            }
            if !found {return Err("source_changed: the selected native shell output is no longer visible".into());}
        }
        Ok(text.trim().into())
    }

    pub fn latest(&self) -> Result<Selection,String> {
        let conn=self.branch.store.connect()?;ready(&self.branch,&conn)?;
        for source in self.branch.sources.iter().rev() {
            let mut before=None;
            loop {
                let found:Option<(String,String)>=conn.query_row(&format!("SELECT r.id,r.order_key FROM rows r WHERE {} AND (?5 IS NULL OR r.order_key<?5) ORDER BY r.order_key DESC LIMIT 1",native_sql(&self.branch)),
                    params![source.id,source.generation,source.segment.end_byte,source.segment.end_ordinal,before],|row|Ok((row.get(0)?,row.get(1)?))).optional().map_err(sql)?;
                let Some((id,order))=found else {break;};before=Some(order);
                match select(self.branch.clone(),&id,Some(&self.branch.generation)) {
                    Err(error) if error.starts_with("rewind_not_candidate:")=>continue,
                    result=>return result,
                }
            }
        }
        Err("detail_unavailable: the native branch contains no rewind target".into())
    }

    pub fn turn_count(&self) -> Result<usize,String> {
        let conn=self.branch.store.connect()?;ready(&self.branch,&conn)?;
        conn.execute_batch("PRAGMA temp_store=FILE; CREATE TEMP TABLE rewind_turns(id TEXT PRIMARY KEY);").map_err(sql)?;
        for (sequence,source) in self.branch.sources.iter().enumerate() {
            let mut records=conn.prepare(&format!("SELECT r.record_start,r.record_end,t.start,t.end FROM rows r LEFT JOIN records t
                ON t.source_id=r.source_id AND t.generation=r.generation AND t.start=(SELECT max(c.start) FROM records c
                    WHERE c.source_id=r.source_id AND c.generation=r.generation AND c.event_type='turn_context' AND c.active=1 AND c.start<=r.record_start)
                WHERE {} AND printf('%04d:',?5)||r.order_key>=?6 ORDER BY r.order_key",native_sql(&self.branch))).map_err(sql)?;
            let records=records.query_map(params![source.id,source.generation,source.segment.end_byte,source.segment.end_ordinal,sequence as u64,self.position],
                |row|Ok((row.get::<_,u64>(0)?,row.get::<_,u64>(1)?,row.get::<_,Option<u64>>(2)?,row.get::<_,Option<u64>>(3)?))).map_err(sql)?;
            for record in records {
                let (from,to,turn,end)=record.map_err(sql)?;verify(&conn,source,from,to)?;
                let (turn,end)=turn.zip(end).ok_or("index_incomplete: a removed user message has no confirmed native turn")?;verify(&conn,source,turn,end)?;
            }
            conn.execute(&format!("INSERT OR IGNORE INTO rewind_turns SELECT coalesce(json_extract(t.projection,'$.payload.turn_id'),json_extract(t.projection,'$.payload.turnId'))
                FROM rows r LEFT JOIN records t ON t.source_id=r.source_id AND t.generation=r.generation AND t.start=(SELECT max(c.start) FROM records c
                    WHERE c.source_id=r.source_id AND c.generation=r.generation AND c.event_type='turn_context' AND c.active=1 AND c.start<=r.record_start)
                WHERE {} AND printf('%04d:',?5)||r.order_key>=?6",native_sql(&self.branch)),
                params![source.id,source.generation,source.segment.end_byte,source.segment.end_ordinal,sequence as u64,self.position]).map_err(sql)?;
        }
        let unknown:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM rewind_turns WHERE id IS NULL)",[],|row|row.get(0)).map_err(sql)?;
        if unknown {return Err("index_incomplete: a removed user message has no confirmed native turn".into());}
        let count:usize=conn.query_row("SELECT count(*) FROM rewind_turns",[],|row|row.get(0)).map_err(sql)?;
        if count==0 {return Err("index_incomplete: native rewind turn range is empty".into());}Ok(count)
    }
}

fn ready(branch:&Branch,conn:&Connection) -> Result<(),String> {
    if let Some(error)=&branch.error {return Err(error.clone());}
    if !matches!(branch.kind,SessionKind::Claude|SessionKind::Codex|SessionKind::Opencode) {
        return Err("detail_unavailable: this provider does not support conversation rewind".into());
    }
    for source in &branch.sources {
        let (generation,status,error):(String,String,Option<String>)=conn.query_row("SELECT generation,status,error FROM sources WHERE id=?1",[&source.id],
            |row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).map_err(sql)?;
        if generation!=source.generation {return Err("source_changed: rewind source generation changed".into());}
        let protocol:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM protocol_sources WHERE source_id=?1)",[&source.id],|row|row.get(0)).map_err(sql)?;
        if !protocol && status!="complete" {return Err(error.unwrap_or_else(||"index_busy: native rewind targets are still being indexed".into()));}
    }Ok(())
}

fn native_sql(branch:&Branch) -> String {
    format!("{} AND r.kind IN ('user','command','shell') AND NOT EXISTS(SELECT 1 FROM protocol_sources WHERE source_id=r.source_id)",branch.visible_sql())
}
fn identity(value:Option<&str>) -> Result<String,String> {
    value.filter(|id|!id.is_empty() && id.len()<=200).map(str::to_owned).ok_or_else(||"index_incomplete: rewind target has no bounded native identity".into())
}
fn verify(conn:&Connection,source:&RegisteredSource,start:u64,end:u64) -> Result<(),String> {
    let mut at=start;
    while at<end {let next=(at+super::limits::CHECKPOINT_BYTES as u64).min(end);api::verified_range(conn,source,at,next)?;at=next;}
    Ok(())
}
fn claude_target(conn:&Connection,source:&RegisteredSource,start:u64,end:u64,value:&Value,validate:bool) -> Result<RewindTarget,String> {
    let mut fields=conn.prepare("SELECT path,decoded_bytes FROM fields WHERE source_id=?1 AND generation=?2 AND record_start=?3
        AND encoding='string' AND (path='/message/content' OR path LIKE '/message/content/%/text')").map_err(sql)?;
    let fields=fields.query_map(params![source.id,source.generation,start],|row|Ok((row.get::<_,String>(0)?,row.get::<_,u64>(1)?))).map_err(sql)?;
    let mut long=false;
    for field in fields {let (path,size)=field.map_err(sql)?;long|=value.pointer(&path).and_then(Value::as_str).is_none_or(|text|text.len() as u64!=size);}
    let raw=value.to_string();let mut own=crate::agent::chat::history::claude_rewind_targets(&raw);
    let mut selected=own.pop().ok_or("rewind_not_candidate: this native record is not a genuine user turn")?;
    if selected.text.starts_with("<bash-stdout>") {
        if let Some(parent)=value["parentUuid"].as_str() {
            let previous:Option<(u64,u64,String)>=conn.query_row("SELECT start,end,projection FROM records WHERE source_id=?1 AND generation=?2
                AND native_id=?3 AND start<?4 AND active=1 AND error IS NULL ORDER BY start DESC LIMIT 1",params![source.id,source.generation,parent,start],
                |row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).optional().map_err(sql)?;
            if let Some((from,to,previous))=previous {
                let previous_value:Value=serde_json::from_str(&previous).map_err(|_|"index_incomplete: native shell input projection is invalid")?;
                if let (Some(input),Some(output))=(super::shell_text::single(conn,source,from,&previous_value)?,super::shell_text::single(conn,source,start,value)?) {
                    if super::shell_text::input(conn,source,&input)?.is_some() && super::shell_text::output(conn,source,&output)?.is_some() {
                        if validate {verify(conn,source,from,to)?;verify(conn,source,start,end)?;}
                        return Err("rewind_not_candidate: native shell output belongs to its input message".into());
                    }
                }
                let pair=crate::agent::chat::history::claude_rewind_targets(&format!("{previous}\n{raw}"));
                if pair.len()==1 && pair[0].last_message_id.as_deref()==Some(selected.message_id.as_str()) {
                    if validate {verify(conn,source,from,to)?;}
                    return Err("rewind_not_candidate: native shell output belongs to its input message".into());
                }
            }
        }
    }
    let next:Option<(u64,u64,String)>=conn.query_row("SELECT start,end,projection FROM records WHERE source_id=?1 AND generation=?2 AND start>?3 AND active=1
        AND error IS NULL AND (?4 IS NULL OR end<=?4) AND (?5 IS NULL OR ordinal IS NULL OR ordinal<?5) ORDER BY start LIMIT 1",
        params![source.id,source.generation,start,source.segment.end_byte,source.segment.end_ordinal],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).optional().map_err(sql)?;
    if let Some((from,to,next))=next {
        let next_value:Value=serde_json::from_str(&next).map_err(|_|"index_incomplete: native shell output projection is invalid")?;
        if selected.text.starts_with("<bash-input>") && next_value["type"]=="user" && next_value["parentUuid"]==selected.message_id
            && next_value["isMeta"]!=true && next_value["isSidechain"]!=true {
            if let (Some(input),Some(output))=(super::shell_text::single(conn,source,start,value)?,super::shell_text::single(conn,source,from,&next_value)?) {
                if super::shell_text::input(conn,source,&input)?.is_some() && super::shell_text::output(conn,source,&output)?.is_some() {
                    if validate {verify(conn,source,from,to)?;verify(conn,source,start,end)?;}
                    selected.last_message_id=Some(identity(next_value["uuid"].as_str())?);selected.text.clear();return Ok(selected);
                }
            }
        }
        let pair=crate::agent::chat::history::claude_rewind_targets(&format!("{raw}\n{next}"));
        if pair.len()==1 && pair[0].last_message_id.is_some() {
            if validate {verify(conn,source,from,to)?;}
            selected=pair.into_iter().next().unwrap();
        }
    }
    if long {selected.text.clear();}
    // A projected preview is a full editable draft only when every contributing string is complete.
    if validate {verify(conn,source,start,end)?;}Ok(selected)
}
fn target(branch:&Branch,conn:&Connection,source:&RegisteredSource,start:u64,end:u64,value:&Value,validate:bool) -> Result<RewindTarget,String> {
    if branch.kind==SessionKind::Claude {return claude_target(conn,source,start,end,value,validate);}
    if validate {verify(conn,source,start,end)?;}
    let (message,turn)=match branch.kind {
        SessionKind::Claude=>(identity(value["uuid"].as_str())?,None),
        SessionKind::Opencode=>(identity(value.pointer(if value.pointer("/info/role").and_then(Value::as_str)==Some("assistant") {"/info/parentID"} else {"/info/id"}).and_then(Value::as_str))?,None),
        SessionKind::Codex=>{
            let turn:Option<(u64,u64,String)>=conn.query_row("SELECT start,end,projection FROM records WHERE source_id=?1 AND generation=?2
                AND event_type='turn_context' AND start<=?3 AND active=1 AND error IS NULL ORDER BY start DESC LIMIT 1",
                params![source.id,source.generation,start],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).optional().map_err(sql)?;
            let (from,to,raw)=turn.ok_or("index_incomplete: native user message has no confirmed turn")?;
            if validate {verify(conn,source,from,to)?;}
            let context:Value=serde_json::from_str(&raw).map_err(|_|"index_incomplete: native turn projection is invalid")?;
            let turn=identity(context.pointer("/payload/turn_id").or_else(||context.pointer("/payload/turnId")).and_then(Value::as_str))?;
            (value.pointer("/payload/id").and_then(Value::as_str).map(str::to_owned).unwrap_or_else(||turn.clone()),Some(turn))
        },
        _=>unreachable!(),
    };
    Ok(RewindTarget {message_id:message,last_message_id:None,turn_id:turn,text:String::new()})
}

pub fn candidates(branch:&Branch,cursor:Option<&str>) -> Result<Value,String> {
    let conn=branch.store.connect()?;ready(branch,&conn)?;
    let from=branch.cursor_sequence("rewind","native",cursor)?;let mut skip=from;let mut examined=0u64;let mut rows=Vec::new();let mut more=false;
    for source in &branch.sources {
        let count:u64=conn.query_row(&format!("SELECT count(*) FROM rows r WHERE {}",native_sql(branch)),
            params![source.id,source.generation,source.segment.end_byte,source.segment.end_ordinal],|row|row.get(0)).map_err(sql)?;
        if skip>=count {skip-=count;continue;}
        let mut statement=conn.prepare(&format!("SELECT r.id,r.summary,p.start,p.end,p.projection FROM rows r JOIN records p
            ON p.source_id=r.source_id AND p.generation=r.generation AND p.start=r.record_start WHERE {} ORDER BY r.order_key LIMIT ?5 OFFSET ?6",native_sql(branch))).map_err(sql)?;
        let values=statement.query_map(params![source.id,source.generation,source.segment.end_byte,source.segment.end_ordinal,61-examined,skip],
            |row|Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?,row.get::<_,u64>(2)?,row.get::<_,u64>(3)?,row.get::<_,String>(4)?))).map_err(sql)?;
        skip=0;
        for value in values {
            let (id,summary,start,end,raw)=value.map_err(sql)?;
            if examined==60 {more=true;break;}
            examined+=1;let value:Value=serde_json::from_str(&raw).map_err(|_|"index_incomplete: native rewind projection is invalid")?;
            match target(branch,&conn,source,start,end,&value,false) {
                Ok(_)=>{let row:Value=serde_json::from_str(&summary).map_err(|e|e.to_string())?;
                    let text=row["text"].as_str().or_else(||row["command"].as_str()).unwrap_or_default().chars().take(240).collect::<String>();
                    rows.push(json!({"rowId":id,"text":text,"at":row["at"],"hasImages":value.pointer("/message/content").or_else(||value.pointer("/payload/content")).and_then(Value::as_array)
                        .is_some_and(|items|items.iter().any(|item|matches!(item["type"].as_str(),Some("image"|"input_image"))))}));},
                Err(error) if error.starts_with("index_incomplete:") || error.starts_with("rewind_not_candidate:")=>{},
                Err(error)=>return Err(error),
            }
        }
        if more {break;}
    }
    let value=json!({"protocolVersion":2,"historyGeneration":branch.generation,"rows":rows,
        "nextCursor":more.then(||branch.cursor("rewind","native",from+examined))});
    if serde_json::to_vec(&value).map_err(|e|e.to_string())?.len()>super::limits::PAGE_BYTES {return Err("unsupported_record: rewind page exceeds its byte budget".into());}
    Ok(value)
}

pub fn select(branch:Branch,row:&str,expected:Option<&str>) -> Result<Selection,String> {
    if expected.is_some_and(|generation|generation!=branch.generation) {return Err("cursor_expired: rewind branch changed".into());}
    let conn=branch.store.connect()?;ready(&branch,&conn)?;let row=branch.resolve_row(&conn,row)?;
    for (sequence,source) in branch.sources.iter().enumerate() {
        let record:Option<(String,u64,u64,String)>=conn.query_row(&format!("SELECT r.order_key,p.start,p.end,p.projection FROM rows r JOIN records p
            ON p.source_id=r.source_id AND p.generation=r.generation AND p.start=r.record_start WHERE {} AND r.id=?5",native_sql(&branch)),
            params![source.id,source.generation,source.segment.end_byte,source.segment.end_ordinal,row],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?))).optional().map_err(sql)?;
        if let Some((order,start,end,raw))=record {
            let value:Value=serde_json::from_str(&raw).map_err(|_|"index_incomplete: native rewind projection is invalid")?;
            let target=target(&branch,&conn,source,start,end,&value,true)?;
            return Ok(Selection {branch,row,target,position:format!("{sequence:04}:{order}")});
        }
    }
    Err("detail_unavailable: user message has no confirmed native rewind identity".into())
}

pub fn prepare(app:&AppCtx,session:&str,cursor:Option<&str>) -> Result<Value,String> {candidates(&api::prepare(app,session)?,cursor)}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn identical_prompts_keep_distinct_native_turns_and_pages_are_scope_bound() {
        let temp=super::super::tests::Temp::new();let mut raw=String::new();
        for number in 0..161 {raw.push_str(&format!("{}\n{}\n",json!({"type":"turn_context","payload":{"turn_id":format!("turn-{number}")}}),
            json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"Identical prompt"}]}})));}
        let branch=super::super::tests::indexed(&temp,&raw);let first=candidates(&branch,None).unwrap();assert_eq!(first["rows"].as_array().unwrap().len(),60);
        let cursor=first["nextCursor"].as_str().unwrap();let next=candidates(&branch,Some(cursor)).unwrap();assert_eq!(next["rows"].as_array().unwrap().len(),60);
        let id=next["rows"][0]["rowId"].as_str().unwrap();assert_eq!(select(branch.clone(),id,Some(&branch.generation)).unwrap().target.turn_id.as_deref(),Some("turn-60"));
        let foreign=Branch {scope:"foreign".into(),..branch.clone()};assert!(candidates(&foreign,Some(cursor)).unwrap_err().starts_with("cursor_expired:"));
        assert!(select(branch.clone(),id,Some("old-generation")).err().unwrap().starts_with("cursor_expired:"));
        assert!(select(branch.clone(),"turn-60",None).is_err());
        let source=&branch.sources[0];let changed=raw.replacen("turn-60","turn-99",1);std::fs::write(&source.segment.path,changed).unwrap();
        assert!(select(branch,id,None).err().unwrap().starts_with("source_changed:"));
    }

    #[test]
    fn full_prefill_uses_original_fields_and_rollback_counts_distinct_native_turns() {
        let temp=super::super::tests::Temp::new();let text="Complete editable original 雪😀\n".repeat(20_000);
        let mut raw=format!("{}\n",json!({"type":"turn_context","payload":{"turn_id":"same-turn"}}));
        for body in [&text,"Steered input"] {raw.push_str(&format!("{}\n",json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":body}]}})));}
        raw.push_str(&format!("{}\n{}\n",json!({"type":"turn_context","payload":{"turn_id":"next-turn"}}),
            json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"Next prompt"}]}})));
        let branch=super::super::tests::indexed(&temp,&raw);let page=candidates(&branch,None).unwrap();let id=page["rows"][0]["rowId"].as_str().unwrap();
        let selected=select(branch,id,None).unwrap();assert_eq!(selected.prefill().unwrap(),text.trim());assert_eq!(selected.turn_count().unwrap(),2);
        assert_eq!(selected.latest().unwrap().target.turn_id.as_deref(),Some("next-turn"));
    }

    #[test]
    fn claude_shell_pair_keeps_its_input_and_final_native_boundary() {
        let temp=super::super::tests::Temp::new();let path=temp.0.join("claude.jsonl");
        let input=json!({"type":"user","uuid":"input","message":{"content":"<bash-input>printf text</bash-input>"}});
        let output=json!({"type":"user","uuid":"output","parentUuid":"input","message":{"content":"<bash-stdout>Literal exit_code=7</bash-stdout>"}});
        std::fs::write(&path,format!("{input}\n{output}\n")).unwrap();let store=super::super::store::Store::at(&temp.0).unwrap();
        let source=store.register(SessionKind::Claude,super::super::source::Segment {path,native_id:"thread".into(),end_byte:None,end_ordinal:None}).unwrap();
        store.scan(SessionKind::Claude,&source,&std::sync::atomic::AtomicBool::new(false),false).unwrap();
        let branch=api::bind(store,vec![source],SessionKind::Claude,"thread".into(),None,"scope").unwrap();let page=candidates(&branch,None).unwrap();
        assert_eq!(page["rows"].as_array().unwrap().len(),1);let selected=select(branch,page["rows"][0]["rowId"].as_str().unwrap(),None).unwrap();
        assert_eq!(selected.target.message_id,"input");assert_eq!(selected.target.last_message_id.as_deref(),Some("output"));
        assert_eq!(selected.latest().unwrap().target.message_id,"input");assert!(selected.prefill().unwrap().contains("Literal exit_code=7"));
    }

    #[test]
    fn long_native_shell_output_is_streamed_and_ambiguous_tags_keep_separate_turns() {
        for ambiguous in [false,true] {
            let temp=super::super::tests::Temp::new();let path=temp.0.join("claude.jsonl");let long="Complete literal output 雪😀\n".repeat(20_000);
            let input=json!({"type":"user","uuid":"input","message":{"content":"<bash-input>printf text</bash-input>"}});
            let output=json!({"type":"user","uuid":"output","parentUuid":"input","message":{"content":format!("<bash-stdout>{long}{}</bash-stdout>",if ambiguous {"<bash-input>literal tag"} else {""})}});
            let raw=format!("{input}\n{output}\n");std::fs::write(&path,&raw).unwrap();let store=super::super::store::Store::at(&temp.0).unwrap();
            let source=store.register(SessionKind::Claude,super::super::source::Segment {path:path.clone(),native_id:"thread".into(),end_byte:None,end_ordinal:None}).unwrap();
            store.scan(SessionKind::Claude,&source,&std::sync::atomic::AtomicBool::new(false),false).unwrap();
            let branch=api::bind(store,vec![source],SessionKind::Claude,"thread".into(),None,"scope").unwrap();let page=candidates(&branch,None).unwrap();
            assert_eq!(page["rows"].as_array().unwrap().len(),if ambiguous {2} else {1});
            let selected=select(branch,page["rows"][0]["rowId"].as_str().unwrap(),None).unwrap();
            assert_eq!(selected.target.last_message_id.is_some(),!ambiguous);
            if !ambiguous {assert_eq!(selected.prefill().unwrap(),format!("<bash-input>printf text</bash-input>\n<bash-stdout>{long}</bash-stdout>"));}
            assert_eq!(std::fs::read_to_string(&path).unwrap(),raw);
        }
    }
}

//! Native step pairing stays in the derived index. User-request ranges exclude runtime annotations.

use std::collections::HashMap;
use std::io::Read;
use rusqlite::{Connection,OptionalExtension,params};
use serde_json::{Value,json};

use super::adapter::{attach,preview,short_input,write_row};
use super::api::DecodedFieldReader;
use super::limits::{INPUT_BYTES,PREVIEW_BYTES};
use super::projection::Record;
use super::store::{RegisteredSource,digest,sql};

fn find(conn:&Connection,source:&RegisteredSource,reference:&str,start:u64,marker:&str) -> Result<Option<u64>,String> {
    let mut reader=DecodedFieldReader::new(conn,source,reference,start,None,0)?;
    let mut bytes=vec![0;INPUT_BYTES];let mut carry=Vec::new();let mut at=start;
    loop {
        let count=reader.read(&mut bytes).map_err(|error|error.to_string())?;
        if count==0 {return Ok(None);}
        let previous=carry.len();carry.extend_from_slice(&bytes[..count]);
        if let Some(found)=carry.windows(marker.len()).position(|window|window==marker.as_bytes()) {return Ok(Some(at-previous as u64+found as u64));}
        at+=count as u64;
        let kept=(marker.len()-1).min(carry.len());carry.drain(..carry.len()-kept);
    }
}

fn visible_user(conn:&Connection,source:&RegisteredSource,record:&Record,refs:&HashMap<String,String>) -> Result<Option<(String,String,u64)>,String> {
    let Some(reference)=refs.get("/content") else {return Ok(None);};
    let Some(field)=record.fields.iter().rev().find(|field|field.path=="/content" && field.encoding=="string") else {return Ok(None);};
    if field.decoded_bytes==0 {return Ok(None);}
    let (start,end)=if let Some(open)=find(conn,source,reference,0,"<USER_REQUEST>")? {
        let start=open+"<USER_REQUEST>".len() as u64;
        let Some(end)=find(conn,source,reference,start,"</USER_REQUEST>")? else {return Ok(None);};
        (start,end)
    } else {(0,field.decoded_bytes)};
    if start==end {return Ok(None);}
    let mut reader=DecodedFieldReader::new(conn,source,reference,start,Some(end),0)?;
    let mut buffer=vec![0;INPUT_BYTES];let mut at=start;let mut first=None;let mut last=start;let mut text=String::new();let mut preview_full=false;
    loop {
        let count=reader.read(&mut buffer).map_err(|error|error.to_string())?;
        if count==0 {break;}
        let chunk=std::str::from_utf8(&buffer[..count]).map_err(|_|"source_changed: user request has invalid UTF-8")?;
        for (offset,ch) in chunk.char_indices() {
            if !ch.is_whitespace() {first.get_or_insert(at+offset as u64);last=at+offset as u64+ch.len_utf8() as u64;}
            if first.is_some() && !preview_full {
                if text.len()+ch.len_utf8()<=PREVIEW_BYTES {text.push(ch);} else {preview_full=true;}
            }
        }
        at+=count as u64;
    }
    let Some(first)=first else {return Ok(None);};
    // A range inside the decoded parent string keeps all source-checkpoint verification. The original
    // content reference is never attached to a user row, so metadata cannot leak into search/download.
    let path="@visible/content";let sliced=digest(format!("{}:{}:{}:{path}:{first}:{last}",source.id,source.generation,record.start).as_bytes());
    conn.execute("INSERT OR REPLACE INTO fields VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,'slice',NULL)",
        params![sliced,source.id,source.generation,record.start,path,first,last,last-first,reference]).map_err(sql)?;
    if last-first<=PREVIEW_BYTES as u64 {text.truncate(((last-first) as usize).min(text.len()));}
    Ok(Some((sliced,text,last-first)))
}

fn paths(record:&Record,root:&str) -> Vec<String> {
    record.fields.iter().filter(|field|field.path==root || field.path.starts_with(&format!("{root}/"))).map(|field|field.path.clone()).collect()
}

pub(super) fn store(conn:&Connection,source:&RegisteredSource,record:&Record,refs:&HashMap<String,String>) -> Result<(),String> {
    let value=&record.value;
    let Some(step)=value["step_index"].as_u64() else {return Ok(());};
    let id=format!("native:{}",digest(format!("antigravity:{}:{step}",source.segment.native_id).as_bytes()));
    let at=crate::agent::chat::history::parsed_at(value["created_at"].as_str());
    match value["type"].as_str() {
        Some("USER_INPUT")=>{
            conn.execute("UPDATE native_pending SET active=0 WHERE source_id=?1 AND generation=?2 AND record_start<?3",params![source.id,source.generation,record.start]).map_err(sql)?;
            if let Some((reference,text,size))=visible_user(conn,source,record,refs)? {
                let row=json!({"kind":"user","id":id,"text":text,"at":at,"bodyAvailable":size>PREVIEW_BYTES as u64});
                write_row(conn,source,record,0,None,&row)?;
                conn.execute("INSERT OR IGNORE INTO row_fields VALUES(?1,?2,?3,?4,'body',0)",params![source.id,source.generation,id,reference]).map_err(sql)?;
            }
        },
        Some("PLANNER_RESPONSE")=>{
            for (local,path,kind) in [(0,"/thinking","reasoning"),(1,"/content","assistant")] {
                if let Some(text)=value.pointer(path).and_then(Value::as_str).filter(|text|!text.is_empty()) {
                    let row_id=if kind=="reasoning" {format!("{id}-thinking")} else {id.clone()};
                    let long=record.fields.iter().any(|field|field.path==path && field.truncated);
                    let row=json!({"kind":kind,"id":row_id,"text":preview(text,PREVIEW_BYTES),"streaming":value["status"]!="DONE","at":at,"bodyAvailable":long});
                    write_row(conn,source,record,local,None,&row)?;attach(conn,source,&row_id,"body",&[path.into()],refs)?;
                }
            }
            for (index,call) in value["tool_calls"].as_array().into_iter().flatten().enumerate() {
                let Some(name)=call["name"].as_str() else {continue;};
                let row_id=format!("{id}-call-{index}");
                let (name,input)=crate::agent::chat::antigravity_protocol::tool_input(name,&call["args"]);
                let row=json!({"kind":"tool","id":row_id,"name":name,"input":short_input(&input),"status":"running","isError":false,"detailAvailable":true});
                write_row(conn,source,record,index+2,Some(&row_id),&row)?;
                attach(conn,source,&row_id,"input",&paths(record,&format!("/tool_calls/{index}/args")),refs)?;
                conn.execute("INSERT OR IGNORE INTO native_pending(source_id,generation,record_start,sequence,row_id) VALUES(?1,?2,?3,?4,?5)",
                    params![source.id,source.generation,record.start,index,row_id]).map_err(sql)?;
            }
        },
        Some("ERROR_MESSAGE"|"GENERIC"|"LIST_DIRECTORY"|"VIEW_FILE"|"CODE_ACTION")=>{
            let pending:Option<(u64,u64,String)>=conn.query_row("SELECT record_start,sequence,row_id FROM native_pending WHERE source_id=?1 AND generation=?2
                AND active=1 AND record_start<?3 ORDER BY record_start,sequence LIMIT 1",params![source.id,source.generation,record.start],
                |row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).optional().map_err(sql)?;
            if let Some((start,sequence,row_id))=pending {
                let error=value["type"]=="ERROR_MESSAGE";
                let row=json!({"kind":"tool","id":row_id,"name":"tool","input":{},"output":preview(value["content"].as_str().unwrap_or_default(),512),
                    "isError":error,"status":if error {"failed"} else if value["status"]=="DONE" {"completed"} else {"running"},"detailAvailable":true});
                write_row(conn,source,record,0,Some(&row_id),&row)?;attach(conn,source,&row_id,"output",&["/content".into()],refs)?;
                conn.execute("UPDATE native_pending SET active=0 WHERE source_id=?1 AND generation=?2 AND record_start=?3 AND sequence=?4",
                    params![source.id,source.generation,start,sequence]).map_err(sql)?;
            } else if value["type"]=="ERROR_MESSAGE" {
                let row=json!({"kind":"error","id":id,"message":preview(value["content"].as_str().unwrap_or_default(),PREVIEW_BYTES)});
                write_row(conn,source,record,0,None,&row)?;attach(conn,source,&id,"body",&["/content".into()],refs)?;
            }
        },
        _=>{},
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::SessionKind;

    fn indexed(temp:&super::super::tests::Temp,records:&[Value]) -> super::super::api::Branch {
        let path=temp.0.join("native.jsonl");
        let raw=records.iter().map(|record|record.to_string()+"\n").collect::<String>();std::fs::write(&path,raw).unwrap();
        let store=super::super::store::Store::at(&temp.0).unwrap();
        let source=store.register(SessionKind::Antigravity,super::super::source::Segment {path,native_id:"native".into(),end_byte:None,end_ordinal:None}).unwrap();
        store.scan(SessionKind::Antigravity,&source,&std::sync::atomic::AtomicBool::new(false),false).unwrap();
        super::super::api::Branch {store,sources:vec![source],generation:"branch".into(),kind:SessionKind::Antigravity,native_id:"native".into(),scope:"scope".into(),error:None,parent:None}
    }

    #[test]
    fn late_wrapped_user_request_excludes_annotations_from_preview_details_and_authorization() {
        let temp=super::super::tests::Temp::new();let request="中文 Ω🚀\n".repeat(15000);
        let content=format!("{}<USER_REQUEST> \n{request}\n </USER_REQUEST><ADDITIONAL_METADATA>private trailing</ADDITIONAL_METADATA>","private prefix ".repeat(5000));
        let branch=indexed(&temp,&[json!({"type":"USER_INPUT","step_index":0,"content":content})]);
        let page=branch.page(None,None,None).unwrap();assert_eq!(page["coverage"],"complete");
        let row=&page["rows"][0];assert_eq!(row["kind"],"user");assert_eq!(row["bodyAvailable"],true);
        assert!(request.starts_with(row["text"].as_str().unwrap()));assert!(!row.to_string().contains("private"));
        let manifest=branch.manifest(row["id"].as_str().unwrap(),0).unwrap();assert_eq!(manifest["fields"].as_array().unwrap().len(),1);
        let reference=manifest["fields"][0]["detailRef"].as_str().unwrap();let mut complete=Vec::new();
        branch.write_detail(reference,None,&mut complete,&std::sync::atomic::AtomicBool::new(false)).unwrap();
        assert_eq!(String::from_utf8(complete).unwrap(),request.trim());
        let conn=branch.store.connect().unwrap();let original:String=conn.query_row("SELECT ref FROM fields WHERE path='/content'",[],|row|row.get(0)).unwrap();
        assert!(branch.detail(&original,None,0).unwrap_err().starts_with("detail_unavailable:"));
        assert!(branch.locate_detail(row["id"].as_str().unwrap(),&original,0).is_err());
    }

    #[test]
    fn native_tool_results_pair_across_records_without_changing_call_identity() {
        let temp=super::super::tests::Temp::new();
        let records=[json!({"type":"USER_INPUT","step_index":0,"content":"<USER_REQUEST>test</USER_REQUEST><ADDITIONAL_METADATA>private</ADDITIONAL_METADATA>"}),
            json!({"type":"PLANNER_RESPONSE","step_index":1,"tool_calls":[{"name":"view_file","args":{"AbsolutePath":"/tmp/test"}}]}),
            json!({"type":"GENERIC","step_index":2,"status":"DONE","content":"file contents"}),
            json!({"type":"PLANNER_RESPONSE","step_index":3,"status":"DONE","content":"answer","thinking":"reason"})];
        let branch=indexed(&temp,&records);let page=branch.page(None,None,None).unwrap();let rows=page["rows"].as_array().unwrap();
        assert_eq!(rows.len(),4);assert_eq!(rows[0]["text"],"test");assert_eq!(rows[1]["name"],"Read");assert_eq!(rows[1]["input"]["file_path"],"/tmp/test");
        assert_eq!(rows[1]["output"],"file contents");assert_eq!(rows[1]["status"],"completed");assert_eq!(rows[2]["text"],"reason");assert_eq!(rows[3]["text"],"answer");
        let manifest=branch.manifest(rows[1]["id"].as_str().unwrap(),0).unwrap();
        assert!(manifest["fields"].as_array().unwrap().iter().any(|field|field["label"]=="input"));
        assert!(manifest["fields"].as_array().unwrap().iter().any(|field|field["label"]=="output"));
        let pending:u64=branch.store.connect().unwrap().query_row("SELECT count(*) FROM native_pending WHERE active=1",[],|row|row.get(0)).unwrap();assert_eq!(pending,0);
    }
}

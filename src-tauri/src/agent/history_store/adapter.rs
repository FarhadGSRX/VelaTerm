//! Provider-specific meaning over a bounded physical-record projection.

use std::collections::HashMap;

use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};

use crate::agent::export::{self, Event};
use crate::models::SessionKind;
use super::limits::PREVIEW_BYTES;
use super::projection::Record;
use super::store::{digest, sql, RegisteredSource};

pub fn expand_arguments(conn:&Connection,source:&RegisteredSource,record:&mut Record,refs:&mut HashMap<String,String>) -> Result<(),String> {
    let outer=record.fields.iter().find(|field|matches!(field.path.as_str(),"/payload/arguments"|"/payload/input") && field.truncated).cloned();
    let Some(outer)=outer else {return Ok(());};
    if !record.value.pointer(&outer.path).and_then(Value::as_str).is_some_and(|text|matches!(text.trim_start().as_bytes().first(),Some(b'{'|b'['))) {return Ok(());}
    let Some(reference)=refs.get(&outer.path).cloned() else {return Ok(());};
    let reader=super::api::DecodedFieldReader::new(conn,source,&reference,0,None,0)?;
    let mut parser=super::projection::Projector::new(reader,0).long_checkpoints_only();
    let prefix=format!("{}/@json",outer.path);
    let mut sink_error=None;
    let inner=parser.next(&mut |chunk| {
        let result=conn.execute("INSERT OR REPLACE INTO checkpoints VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
            params![source.id,source.generation,record.start,format!("{prefix}{}",chunk.path),chunk.raw_start,chunk.raw_end,
                chunk.decoded_start,chunk.decoded_end,chunk.hash,chunk.sequence]).map_err(sql);
        if let Err(error)=&result {sink_error=Some(error.clone());}
        result.map(|_|())
    })?;
    if let Some(error)=sink_error {return Err(error);}
    let Some(inner)=inner else {return Ok(());};
    if inner.error.is_some() {
        // A tool is allowed to take ordinary string arguments. Keep their original detail reference.
        // A malformed inner object is never retried through an unbounded serde_json allocation.
        return Ok(());
    }
    if !inner.value.is_object() && !inner.value.is_array() {return Ok(());}
    for mut field in inner.fields {
        field.path=format!("{prefix}{}",field.path);
        let inner_ref=field_ref(&format!("{}:{}:{}:{}",source.id,source.generation,record.start,field.path));
        conn.execute("INSERT OR REPLACE INTO fields VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",params![inner_ref,source.id,source.generation,
            record.start,field.path,field.start,field.end,field.decoded_bytes,reference,field.encoding,field.hash]).map_err(sql)?;
        refs.insert(field.path.clone(),inner_ref);
        record.fields.push(field);
    }
    if let Some(value)=record.value.pointer_mut(&outer.path) {*value=inner.value;}
    Ok(())
}

pub fn preview(text: &str, bytes: usize) -> String {
    let mut end = text.len().min(bytes);
    while !text.is_char_boundary(end) { end -= 1; }
    text[..end].to_owned()
}

/// Subtrees no reader addresses field by field. Codex writes the whole conversation so far into
/// every compaction record; indexing those copies again made a long session's index several times
/// its own size. The record projection still carries the record for classification.
const UNADDRESSED_SUBTREES:[&str;1]=["/payload/replacement_history"];
pub(super) fn addressed(path:&str) -> bool {!UNADDRESSED_SUBTREES.iter().any(|subtree|path.starts_with(subtree))}

/// A field reference: 128 bits of the field's identity. It is stored on every field and row-field
/// row, so its width is a large part of the index.
pub(super) fn field_ref(identity:&str) -> String {let mut reference=digest(identity.as_bytes());reference.truncate(32);reference}

/// Codex records that only carry usage, settings or lifecycle state. Readers take them from the
/// record projection and never address their strings, so they get no field rows.
fn metadata_only(value:&Value) -> bool {
    match value["type"].as_str() {
        Some("token_usage_record"|"world_state"|"turn_context"|"session_meta")=>true,
        Some("event_msg")=>matches!(value.pointer("/payload/type").and_then(Value::as_str),
            Some("token_count"|"thread_settings_applied"|"task_started"|"task_complete")),
        _=>false,
    }
}

/// The projection kept for a record. A compaction record repeats the whole conversation so far;
/// only its type marks the compaction, so the repeated history is left out.
pub(super) fn stored_projection(value:&Value) -> std::borrow::Cow<'_,Value> {
    if value["type"]=="compacted" && value.pointer("/payload/replacement_history").is_some() {
        let mut value=value.clone();
        if let Some(payload)=value["payload"].as_object_mut() {payload.remove("replacement_history");payload.remove("replacement_history_metadata");}
        std::borrow::Cow::Owned(value)
    } else {std::borrow::Cow::Borrowed(value)}
}

pub fn store_fields(conn: &Connection, source: &RegisteredSource, record: &Record) -> Result<HashMap<String, String>, String> {
    let mut refs = HashMap::new();
    let mut latest=HashMap::new();
    if metadata_only(&record.value) {return Ok(refs);}
    for field in record.fields.iter().filter(|field|addressed(&field.path)) {latest.insert(&field.path,field);}
    for field in latest.values() {
        let reference = field_ref(&format!("{}:{}:{}:{}",source.id,source.generation,record.start,field.path));
        conn.execute("INSERT OR REPLACE INTO fields(ref,source_id,generation,record_start,path,raw_start,raw_end,decoded_bytes,encoding,hash)
            VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)", params![reference,source.id,source.generation,record.start,
            field.path,field.start,field.end,field.decoded_bytes,field.encoding,field.hash]).map_err(sql)?;
        refs.insert(field.path.clone(), reference);
        if field.encoding=="string" {
            conn.execute("DELETE FROM checkpoints WHERE source_id=?1 AND generation=?2 AND record_start=?3 AND path=?4
                AND (raw_start<?5 OR raw_end>?6)",params![source.id,source.generation,record.start,field.path,field.start,field.end]).map_err(sql)?;
        }
    }
    Ok(refs)
}

fn get<'a>(value: &'a Value, names: &[&str]) -> Option<&'a str> {
    names.iter().find_map(|key| value.get(*key).and_then(Value::as_str))
}

pub(crate) fn short_input(input: &Value) -> Value {
    let mut object = serde_json::Map::new();
    for key in ["command","description","path","file_path","notebook_path","pattern","url","query","skill","name","subagent_type","agentThreadId"] {
        if let Some(value) = input.get(key).and_then(Value::as_str) {
            object.insert(key.into(), Value::String(preview(value, 512)));
        }
    }
    Value::Object(object)
}

pub(crate) fn body_paths(record: &Record, role: &str, call: Option<&str>, local:usize) -> Vec<String> {
    if record.value["method"]=="session/update" {return vec!["/params/update/content/text".into()];}
    if record.value["type"]=="kiro_pair" {
        return vec![if local==0 {"/user/content/Prompt/prompt"} else {"/assistant/Response/content"}.into()];
    }
    if record.value["type"]=="kiro_message" {
        return record.fields.iter().filter(|field|field.encoding=="string" && field.path.starts_with("/data/content/") && field.path.ends_with("/data"))
            .map(|field|field.path.clone()).collect();
    }
    if matches!(record.value["type"].as_str(),Some("opencode_message"|"opencode_part")) {return super::native_database::body_paths(record,role);}
    if record.value["type"].as_str() == Some("response_item") {
        return record.fields.iter().filter(|field| field.encoding=="string" || role=="toolInput").filter(|field| match role {
            "toolInput" => field.path.starts_with("/payload/arguments") || field.path.starts_with("/payload/input"),
            "toolOutput" => field.path.starts_with("/payload/output"),
            "reasoning" => field.path.starts_with("/payload/summary/") && field.path.ends_with("/text"),
            _ => field.path.starts_with("/payload/content/") && field.path.ends_with("/text"),
        }).map(|field| field.path.clone()).collect();
    }
    if let Some(call) = call {
        if let Some(blocks) = record.value.pointer("/message/content").and_then(Value::as_array) {
            for (index, block) in blocks.iter().enumerate() {
                if get(block, &["id","tool_use_id","toolCallId"]) == Some(call) {
                    let root = format!("/message/content/{index}/{}", if role == "toolInput" {
                        if block.get("input").is_some() { "input" } else { "arguments" }
                    } else { "content" });
                    return record.fields.iter().filter(|field| field.path == root || field.path.starts_with(&(root.clone()+"/")) &&
                        (role=="toolInput" || field.encoding=="string" && field.path.ends_with("/text")))
                        .map(|field| field.path.clone()).collect();
                }
            }
        }
    }
    if (record.value["type"]=="assistant" || record.value["type"]=="message" && record.value.pointer("/message/role").and_then(Value::as_str)==Some("assistant")) && call.is_none() {
        if let Some(blocks)=record.value.pointer("/message/content").and_then(Value::as_array) {
            let selected=blocks.iter().enumerate().filter(|(_,block)|match block["type"].as_str() {
                Some("tool_use"|"toolCall")=>true,
                Some("text")=>block["text"].as_str().is_some_and(|text|!text.trim().is_empty() && text.trim()!=export::NO_RESPONSE_PLACEHOLDER),
                Some("thinking")=>block["thinking"].as_str().is_some_and(|text|!text.trim().is_empty()),
                _=>false,
            }).nth(local).map(|(index,_)|index);
            if let Some(index)=selected {
                let root=format!("/message/content/{index}/{}",if role=="reasoning" {"thinking"} else if role=="toolInput" {"input"} else {"text"});
                return record.fields.iter().filter(|field|field.path==root || (role=="toolInput" && field.path.starts_with(&(root.clone()+"/"))))
                    .map(|field|field.path.clone()).collect();
            }
        }
    }
    record.fields.iter().filter(|field| field.encoding=="string" || role=="toolInput").filter(|field| match role {
        "toolInput" => field.path.contains("/input/") || field.path.contains("/arguments/"),
        "toolOutput" => field.path.starts_with("/message/content") || field.path == "/message/output",
        "reasoning" => field.path.starts_with("/message/content/") && field.path.ends_with("/thinking"),
        _ => field.path == "/message/content" || (field.path.starts_with("/message/content/") && field.path.ends_with("/text")) || field.path == "/content",
    }).map(|field| field.path.clone()).collect()
}

pub(super) fn attach(conn: &Connection, source: &RegisteredSource, row: &str, label: &str, paths: &[String], refs: &HashMap<String,String>) -> Result<(), String> {
    let base: u64 = conn.query_row("SELECT coalesce(max(sequence)+1,0) FROM row_fields WHERE source_id=?1 AND generation=?2 AND row_id=?3",
        params![source.id,source.generation,row], |r| r.get(0)).map_err(sql)?;
    for (index, path) in paths.iter().enumerate() {
        if let Some(reference) = refs.get(path) {
            conn.execute("INSERT OR IGNORE INTO row_fields VALUES(?1,?2,?3,?4,?5,?6)",
                params![source.id,source.generation,row,reference,label,base+index as u64]).map_err(sql)?;
        }
    }
    Ok(())
}

pub(super) fn write_row(conn: &Connection, source: &RegisteredSource, record: &Record, local: usize, call: Option<&str>, value: &Value) -> Result<(), String> {
    conn.execute("INSERT OR REPLACE INTO row_parts VALUES(?1,?2,?3,?4,?5,?6,?7)",params![source.id,source.generation,value["id"].as_str().unwrap(),
        record.start,record.end,record.value["ordinal"].as_u64(),value.to_string()]).map_err(sql)?;
    conn.execute("INSERT INTO rows(source_id,generation,id,order_key,record_start,record_end,ordinal,kind,call_id,summary)
        VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)
        ON CONFLICT(source_id,generation,id) DO UPDATE SET summary=excluded.summary,revision=rows.revision+1,
        record_end=CASE WHEN excluded.record_start<rows.record_start THEN excluded.record_end ELSE rows.record_end END,
        ordinal=CASE WHEN excluded.record_start<rows.record_start THEN excluded.ordinal ELSE rows.ordinal END,
        record_start=min(rows.record_start,excluded.record_start),order_key=min(rows.order_key,excluded.order_key)",
        params![source.id,source.generation,value["id"].as_str().unwrap(),format!("{:020}:{local:06}",record.start),record.start,record.end,
            record.value["ordinal"].as_u64(),value["kind"].as_str().unwrap(),call,value.to_string()]).map_err(sql)?;
    if value["kind"]=="user" {super::attachments::attach(conn,source,record,value["id"].as_str().unwrap())?;}
    Ok(())
}

pub fn store_record(conn: &Connection, kind: SessionKind, source: &RegisteredSource, record: &Record, refs: &HashMap<String,String>) -> Result<(), String> {
    let value = &record.value;
    let native = match kind {
        SessionKind::Claude => get(value, &["uuid"]),
        SessionKind::Pi | SessionKind::Omp => get(value, &["id"]),
        _ => value.pointer("/payload/id").and_then(Value::as_str),
    };
    let parent = get(value, &["parentUuid","logicalParentUuid","parentId"]);
    // A record seen again by a scan resumed from an earlier watermark keeps its branch state: only
    // the scan that reaches the leaf recomputes `active`, and resetting it here would let an
    // abandoned turn reappear until then.
    conn.execute("INSERT INTO records(source_id,generation,start,end,ordinal,native_id,parent_id,event_type,projection,error)
        VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10) ON CONFLICT(source_id,generation,start) DO UPDATE SET end=excluded.end,ordinal=excluded.ordinal,
        native_id=excluded.native_id,parent_id=excluded.parent_id,event_type=excluded.event_type,projection=excluded.projection,error=excluded.error", params![source.id,source.generation,record.start,record.end,
            value["ordinal"].as_u64(),native,parent,value["type"].as_str(),stored_projection(value).to_string(),record.error]).map_err(sql)?;
    if let Some(error) = &record.error {
        let row = json!({"kind":"notice","id":format!("gap:{}:{}:{}",source.id,source.generation,record.start),"message":error});
        return write_row(conn,source,record,0,None,&row);
    }
    if kind==SessionKind::Antigravity {return super::antigravity_adapter::store(conn,source,record,refs);}
    let mut projected=value.clone();
    if super::children::is_claude_child(source) {projected["isSidechain"]=json!(false);}
    let raw = projected.to_string();
    let events = match kind {
        SessionKind::Codex => export::codex_events(&raw),
        SessionKind::Claude => export::claude_events(&raw),
        SessionKind::Pi | SessionKind::Omp => export::pi_events(&raw),
        SessionKind::Grok=>crate::agent::transcript::grok_pieces(&projected).into_iter().map(|piece| {
            if let Some(name)=piece.tools.first() {Event::ToolUse {id:None,name:name.clone(),input:json!({}),ts:piece.timestamp}}
            else if piece.role=="user" {Event::User {text:piece.text,ts:piece.timestamp}}
            else {Event::AssistantText {text:piece.text,ts:piece.timestamp}}
        }).collect(),
        _ => return Err("Bounded history projection has no provider adapter".into()),
    };
    for (local, event) in events.into_iter().enumerate() {
        let physical = format!("{}:{}:{}:{}:{local}",kind.as_str(),source.id,source.generation,record.start);
        let mut call_id = None;
        let mut at = None;
        let mut label = "body";
        let mut role = "body";
        let mut row = match event {
            Event::User { text, ts } => { at=ts; json!({"kind":"user","text":preview(&text,PREVIEW_BYTES)}) }
            Event::AssistantText { text, ts } => { at=ts; json!({"kind":"assistant","text":preview(&text,PREVIEW_BYTES),"streaming":false}) }
            Event::Thinking { text, .. } => { role="reasoning"; json!({"kind":"reasoning","text":preview(&text,PREVIEW_BYTES),"streaming":false}) }
            Event::Command { text, ts } => { at=ts; json!({"kind":"command","text":preview(&text,PREVIEW_BYTES)}) }
            Event::Wake { summary, ts } => { at=ts; json!({"kind":"wake","summary":preview(&summary,PREVIEW_BYTES)}) }
            Event::Shell { command,output,exit_code,cancelled,truncated,ts,.. } => { at=ts; json!({"kind":"shell","command":preview(&command,PREVIEW_BYTES),
                "stdout":preview(&output,PREVIEW_BYTES),"stderr":"","stdoutTruncated":truncated,"stderrTruncated":false,
                "status":if cancelled {"cancelled"} else {"completed"},"exitCode":exit_code}) }
            Event::ToolUse { id,name,input,ts } => {
                call_id=id; at=ts; label="input"; role="toolInput";
                json!({"kind":"tool","name":name,"input":short_input(&input),"isError":false,"status":"running","detailAvailable":true})
            }
            Event::ToolResult { id,name,text,is_error } => {
                call_id=id; label="output"; role="toolOutput";
                json!({"kind":"tool","name":name.unwrap_or_else(||"tool".into()),"input":{},"output":preview(&text,512),
                    "isError":is_error,"status":if is_error {"failed"} else {"completed"},"detailAvailable":true})
            }
        };
        let id = if let Some(call) = &call_id { format!("call:{}",digest(format!("{}:{}:{call}",kind.as_str(),source.segment.native_id).as_bytes())) }
            else if let Some(native) = native { format!("native:{}",digest(format!("{}:{}:{native}:{local}",kind.as_str(),source.segment.native_id).as_bytes())) }
            else { format!("record:{}",digest(physical.as_bytes())) };
        row["id"] = Value::String(id.clone());
        if let Some(at) = crate::agent::chat::history::parsed_at(at.as_deref()) { row["at"] = json!(at); }
        let paths = body_paths(record,role,call_id.as_deref(),local);
        let long = paths.iter().any(|path| record.fields.iter().any(|field| &field.path==path && field.truncated));
        if long && row["kind"] != "tool" { row["bodyAvailable"] = json!(true); }
        super::children::annotate(kind,record,&mut row)?;
        write_row(conn,source,record,local,call_id.as_deref(),&row)?;
        attach(conn,source,&id,label,&paths,refs)?;
        if kind==SessionKind::Codex {
            if row["kind"]=="user" {super::protocol_source::set_state(conn,source,CODEX_PENDING_USER,&id)?;}
            if let Some(call)=&call_id {
                let open=format!("{call}\n{id}");
                if role=="toolInput" {super::protocol_source::set_state(conn,source,CODEX_OPEN_CALL,&open)?;}
                else if super::protocol_source::state(conn,source,CODEX_OPEN_CALL)?.as_deref()==Some(open.as_str()) {
                    // The bracket closed; a later completed item belongs to another call.
                    super::protocol_source::set_state(conn,source,CODEX_OPEN_CALL,"")?;
                }
            }
        }
    }
    // Completed native items carry historical diffs that response_item does not include.
    if kind == SessionKind::Codex && matches!(value.pointer("/payload/type").and_then(Value::as_str), Some("item_completed" | "itemCompleted")) {
        if let Some(item) = value.pointer("/payload/item") {
            supersede_codex_response(conn,source,item)?;
            super::children::codex_record(conn,source,record,item)?;
            if item["type"].as_str().is_some_and(|kind|kind.replace('_',"").eq_ignore_ascii_case("userMessage")) {
                super::protocol_source::codex_user(conn,source,record,refs,"/payload/item")?;
            }
            if let Some(mut row) = crate::agent::chat::history::recorded_codex_item_row(item,value["timestamp"].as_str())
                .and_then(|row|serde_json::to_value(row).ok()) {
                let native = row["id"].as_str().unwrap_or_default().to_owned();
                let id = if row["kind"]=="assistant" || row["kind"]=="reasoning" {
                    format!("native:{}",digest(format!("{}:{}:{native}:0",kind.as_str(),source.segment.native_id).as_bytes()))
                } else {format!("item:{}",digest(format!("{}:{}:{native}",kind.as_str(),source.segment.native_id).as_bytes()))};
                row["id"]=json!(id);
                row["detailAvailable"]=json!(true);
                if row["kind"]=="tool" {
                    row["input"]=short_input(&row["input"]);
                    if let Some(output)=row["output"].as_str() {row["output"]=json!(preview(output,512));}
                }
                let category=item["type"].as_str().unwrap_or_default().replace('_',"").to_ascii_lowercase();
                let roots:Vec<&str>=match category.as_str() {
                    "agentmessage"=>vec!["content"],
                    "reasoning"=>["summary_text","summaryText","summary","raw_content"].into_iter().find(|key|item.get(key).is_some()).into_iter().collect(),
                    "commandexecution"=>vec!["command","cwd"],
                    "filechange"=>vec!["changes"],
                    "mcptoolcall"|"dynamictoolcall"=>vec!["arguments"],
                    "websearch"=>vec!["query"],
                    _=>Vec::new(),
                };
                let paths:Vec<_>=record.fields.iter().filter(|field|roots.iter().any(|root| {
                    let prefix=format!("/payload/item/{root}");field.path==prefix || field.path.starts_with(&(prefix+"/"))
                }) && (row["kind"]=="tool" || field.encoding=="string" && (field.path.ends_with("/text") || !field.path.contains("/content/"))))
                    .map(|field|field.path.clone()).collect();
                if row["kind"]!="tool" && record.fields.iter().any(|field|paths.contains(&field.path) && field.truncated) {
                    row["bodyAvailable"]=json!(true);
                }
                write_row(conn,source,record,0,Some(&native),&row)?;
                attach(conn,source,&id,if row["kind"]=="tool" {"input"} else {"body"},&paths,refs)?;
                let outputs:Vec<_>=match category.as_str() {
                    "commandexecution"=>vec!["aggregated_output","aggregatedOutput","formatted_output","stdout","stderr"],
                    "mcptoolcall"|"dynamictoolcall"=>vec!["result","error"],
                    "websearch"=>vec!["results"],
                    _=>Vec::new(),
                };
                let paths:Vec<_>=record.fields.iter().filter(|field|outputs.iter().any(|root| {
                    let prefix=format!("/payload/item/{root}");field.path==prefix || field.path.starts_with(&(prefix+"/"))
                })).map(|field|field.path.clone()).collect();
                attach(conn,source,&id,"output",&paths,refs)?;
                store_files(conn,source,&id,record.start,item,refs)?;
            }
        }
    }
    Ok(())
}

const CODEX_PENDING_USER:&str="codex:pendingUser";
const CODEX_OPEN_CALL:&str="codex:openCall";

/// Newer Codex recordings write each turn twice: response items for the model and completed items
/// for the client. The live view shows the completed items, so their response-item copies are
/// superseded: the user message just before a completed user message, and a generic call whose
/// bracket holds a completed tool item. Superseded rows stay addressable, because transcript prose,
/// exports and search still read the response items and authorize their fields through these rows.
fn supersede_codex_response(conn:&Connection,source:&RegisteredSource,item:&Value) -> Result<(),String> {
    use super::protocol_source::{item_id,set_state,state};
    let category=item["type"].as_str().unwrap_or_default().replace('_',"").to_ascii_lowercase();
    let Some(native)=item["id"].as_str() else {return Ok(());};
    let supersede=|id:&str,replacement:&str| -> Result<(),String> {
        conn.execute("INSERT OR REPLACE INTO superseded_rows VALUES(?1,?2,?3,?4)",params![source.id,source.generation,id,replacement]).map_err(sql)?;Ok(())
    };
    if category=="usermessage" {
        if let Some(id)=state(conn,source,CODEX_PENDING_USER)?.filter(|id|!id.is_empty()) {
            supersede(&id,&item_id(SessionKind::Codex,source,native,true))?;set_state(conn,source,CODEX_PENDING_USER,"")?;
        }
    }
    if matches!(category.as_str(),"filechange"|"commandexecution"|"mcptoolcall"|"dynamictoolcall"|"websearch") {
        if let Some((_,id))=state(conn,source,CODEX_OPEN_CALL)?.as_deref().and_then(|value|value.split_once('\n')) {
            supersede(id,&item_id(SessionKind::Codex,source,native,false))?;
        }
    }
    Ok(())
}

pub(super) fn store_files(conn: &Connection, source: &RegisteredSource, row: &str, record_start:u64, item: &Value, refs: &HashMap<String,String>) -> Result<(), String> {
    store_files_at(conn,source,row,record_start,item,refs,"/payload/item/changes")
}

pub(super) fn store_files_at(conn:&Connection,source:&RegisteredSource,row:&str,record_start:u64,item:&Value,refs:&HashMap<String,String>,root:&str) -> Result<(),String> {
    let changes = match item.get("changes") {
        Some(Value::Array(array)) => array.iter().enumerate().map(|(index,value)|(index.to_string(),value)).collect::<Vec<_>>(),
        Some(Value::Object(object)) => object.iter().map(|(key,value)|(key.clone(),value)).collect::<Vec<_>>(),
        _ => return Ok(()),
    };
    for (index,(key,change)) in changes.into_iter().enumerate() {
        let path = get(change,&["path","file_path"]).unwrap_or(&key);
        let root = format!("{root}/{}",key.replace('~',"~0").replace('/',"~1"));
        let detail = ["unified_diff","unifiedDiff","diff","content"].into_iter().find_map(|name| {
            refs.get(&format!("{root}/{name}")).map(|reference|(reference,name))
        });
        let change_type=get(change,&["type","kind"]).or_else(||change.pointer("/kind/type").and_then(Value::as_str));
        conn.execute("INSERT OR REPLACE INTO files VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",params![source.id,source.generation,row,index as u64,
            preview(path,1024),change_type,detail.map(|d|d.0),detail.map(|d|if d.1=="content" {"text"} else {"diff"}),record_start]).map_err(sql)?;
    }
    Ok(())
}

pub fn select_branch(conn: &Connection, kind: SessionKind, source: &RegisteredSource) -> Result<(), String> {
    if !matches!(kind,SessionKind::Claude|SessionKind::Pi|SessionKind::Omp) { return Ok(()); }
    let latest: Option<(String,Option<String>)> = conn.query_row("SELECT projection,native_id FROM records WHERE source_id=?1 AND generation=?2
        AND (native_id IS NOT NULL OR (event_type='last-prompt' AND json_extract(projection,'$.rewound')=1)) ORDER BY start DESC LIMIT 1",params![source.id,source.generation],|r|Ok((r.get(0)?,r.get(1)?)))
        .optional().map_err(sql)?;
    let Some((projection,native))=latest else { return Ok(()); };
    let value:Value=serde_json::from_str(&projection).map_err(|e|e.to_string())?;
    let leaf=if value["type"]=="last-prompt" && value["rewound"]==true { value["leafUuid"].as_str().map(str::to_owned) } else { native };
    // Each step looks its parent up through record_identity. Left to the planner, the join scans
    // every record of the source per step, which made a long conversation quadratic.
    let chain="WITH RECURSIVE chain(id) AS (SELECT ?3 UNION SELECT r.parent_id FROM chain c CROSS JOIN records r ON r.native_id=c.id
        WHERE r.source_id=?1 AND r.generation=?2 AND r.parent_id IS NOT NULL)";
    if leaf.is_some() {
        let (nodes,root,missing):(u64,bool,bool)=conn.query_row(&format!("{chain} SELECT (SELECT count(*) FROM chain),
            EXISTS(SELECT 1 FROM chain c CROSS JOIN records r ON r.native_id=c.id WHERE r.source_id=?1 AND r.generation=?2 AND r.parent_id IS NULL),
            EXISTS(SELECT 1 FROM chain c WHERE NOT EXISTS(SELECT 1 FROM records r WHERE r.source_id=?1 AND r.generation=?2 AND r.native_id=c.id))"),
            params![source.id,source.generation,leaf],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(sql)?;
        if nodes>100000 || !root || missing {return Err("index_incomplete: native history branch is cyclic, too deep, or missing an ancestor".into());}
    }
    let selected=if super::children::is_claude_child(source) {
        format!("{chain}, kept(id,restored) AS (SELECT id,0 FROM chain UNION
            SELECT r.native_id,1 FROM kept k CROSS JOIN records r ON r.parent_id=k.id
            JOIN records parent ON parent.source_id=r.source_id AND parent.generation=r.generation AND parent.native_id=r.parent_id
            WHERE r.source_id=?1 AND r.generation=?2 AND r.native_id IS NOT NULL AND (
                (r.event_type='assistant' AND json_type(r.projection,'$.message.id')='text'
                    AND json_extract(r.projection,'$.message.id')=json_extract(parent.projection,'$.message.id'))
                OR (k.restored=1 AND (r.event_type='attachment' OR (r.event_type='user'
                    AND json_type(r.projection,'$.message.content')='array' AND json_array_length(r.projection,'$.message.content')>0
                    AND NOT EXISTS(SELECT 1 FROM json_each(r.projection,'$.message.content') b WHERE coalesce(json_extract(b.value,'$.type'),'')!='tool_result'))))))")
    } else {chain.to_owned()};
    let selected_table=if super::children::is_claude_child(source) {"kept"} else {"chain"};
    let removed:bool=conn.query_row(&format!("{selected} SELECT EXISTS(SELECT 1 FROM records WHERE source_id=?1 AND generation=?2
        AND active=1 AND native_id IS NOT NULL AND native_id NOT IN (SELECT id FROM {selected_table} WHERE id IS NOT NULL))"),
        params![source.id,source.generation,leaf],|r|r.get(0)).map_err(sql)?;
    if removed {conn.execute("UPDATE sources SET branch_revision=?3 WHERE id=?1 AND generation=?2",
        params![source.id,source.generation,uuid::Uuid::new_v4().to_string()]).map_err(sql)?;}
    conn.execute(&format!("{selected} UPDATE records SET active=CASE WHEN native_id IS NULL OR native_id IN (SELECT id FROM {selected_table}) THEN 1 ELSE 0 END
        WHERE source_id=?1 AND generation=?2"),params![source.id,source.generation,leaf]).map_err(sql)?;
    Ok(())
}

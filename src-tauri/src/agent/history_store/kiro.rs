//! Kiro conversations, read through Kiro's verified history parsers into in-memory records.
//!
//! The native session file pair and database are only read; one record per message is laid out in
//! memory, and laid out again only when one of the native files changed.

use std::path::PathBuf;
use serde_json::{json,Value};

use crate::agent::kiro_store;
use crate::models::SessionKind;
use super::adapter::{self,Refs,Scanned};
use super::limits::PREVIEW_BYTES;
use super::projection::Record;
use super::store::{digest,RegisteredSource,State,Store};

/// Stamps of every native file the conversation can be read from.
fn fingerprint(id:&str) -> Result<String,String> {
    let roots=kiro_store::Roots::native()?;
    let mut facts=Vec::new();
    for path in [roots.sessions.join(format!("{id}.json")),roots.sessions.join(format!("{id}.jsonl"))] {
        facts.push(match std::fs::metadata(&path) {
            Ok(metadata)=>json!([super::source::identity(&metadata),metadata.len(),format!("{:?}",metadata.modified().ok())]),
            Err(_)=>Value::Null,
        });
    }
    if let Some(database)=roots.database.filter(|path|path.is_file()) {facts.push(json!(super::opencode::stamp(&database)?));}
    Ok(json!(facts).to_string())
}

fn layout(id:&str) -> Result<Vec<u8>,String> {
    let session=kiro_store::read(id).map_err(|error|format!("source_missing: {error}"))?;
    let messages=session.messages.map_err(|error|format!("unsupported_record: {error}"))?;
    let mut bytes=Vec::new();
    for (index,message) in messages.into_iter().enumerate() {
        let native=message.native_id.unwrap_or_else(||format!("{}{index}",message.role));
        serde_json::to_writer(&mut bytes,&json!({"type":"kiro_message","id":native,"role":message.role,"text":message.text,"timestamp":message.timestamp}))
            .map_err(|e|e.to_string())?;bytes.push(b'\n');
    }
    Ok(bytes)
}

pub(super) fn register(store:&Store,path:PathBuf,native:&str) -> Result<RegisteredSource,String> {
    if !kiro_store::valid_id(native) {return Err("unsupported_record: Kiro session identity is invalid".into());}
    let fingerprint=fingerprint(native)?;
    store.register_virtual(SessionKind::Kiro,&format!("kiro:{native}"),&path,native,fingerprint,||layout(native))
}

pub(super) fn store_record(state:&mut State,scanned:&Scanned,record:&Record,refs:&Refs) -> Result<(),String> {
    let value=&record.value;
    if value["type"]!="kiro_message" {return Ok(());}
    let role=if value["role"]=="user" {"user"} else {"assistant"};
    let native=value["id"].as_str().unwrap_or_default();
    let id=format!("native:{}",digest(format!("kiro:{}:{native}:{role}",scanned.source.segment.native_id).as_bytes()));
    let field=record.fields.iter().find(|field|field.path=="/text" && field.encoding=="string").ok_or("unsupported_record: Kiro body range is unavailable")?;
    let mut row=json!({"kind":role,"id":id,"text":adapter::preview(value["text"].as_str().unwrap_or_default(),PREVIEW_BYTES),"streaming":false,"bodySizeBytes":field.decoded_bytes});
    if field.truncated {row["bodyAvailable"]=json!(true);}
    if let Some(at)=crate::agent::chat::history::parsed_at(value["timestamp"].as_str()) {row["at"]=json!(at);}
    adapter::write_row(state,scanned,record,0,None,&row,refs);adapter::attach(state,&id,"body",&["/text".into()],refs);Ok(())
}

/// The model and context use Kiro reports for a conversation.
pub(super) fn context(id:&str) -> Result<kiro_store::Context,String> {
    let session=kiro_store::read(id)?;session.messages.map_err(|error|format!("index_incomplete: {error}"))?;Ok(session.context)
}

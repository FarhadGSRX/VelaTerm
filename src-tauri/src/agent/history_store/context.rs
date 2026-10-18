//! Info-panel queries use the audited, bounded metadata of the current generation.

use rusqlite::{params,OptionalExtension};
use serde_json::Value;
use crate::{agent::kiro_store,host::AppCtx,models::SessionKind};
use super::{api::{self,Branch},store::sql};

pub fn kiro(app:&AppCtx,session:&str) -> Result<kiro_store::Context,String> {kiro_branch(&api::prepare(app,session)?)}

pub(super) fn kiro_branch(branch:&Branch) -> Result<kiro_store::Context,String> {
    if branch.kind!=SessionKind::Kiro {return Err("unsupported_record: Kiro context requires a Kiro session".into());}
    if let Some(error)=&branch.error {return Err(error.clone());}
    let conn=branch.store.connect()?;
    let source=branch.sources.first().ok_or("source_missing: Kiro history is unavailable")?;
    let (generation,status,error):(String,String,Option<String>)=conn.query_row("SELECT generation,status,error FROM sources WHERE id=?1",
        [&source.id],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).map_err(sql)?;
    if generation!=source.generation {return Err("source_changed: Kiro history generation changed".into());}
    if status=="error" {return Err(error.unwrap_or_else(||"index_incomplete: Kiro history could not be verified".into()));}
    if status!="complete" {return Err("index_busy: Kiro context is awaiting verified history metadata".into());}
    let metadata=source.kiro_pair.as_ref().map(|pair|&pair.metadata).unwrap_or(source);
    let raw:Option<String>=conn.query_row("SELECT projection FROM records WHERE source_id=?1 AND generation=?2 AND event_type='kiro_metadata' AND active=1",
        params![metadata.id,metadata.generation],|row|row.get(0)).optional().map_err(sql)?;
    let raw=raw.ok_or("index_incomplete: verified Kiro context metadata is unavailable")?;
    let value:Value=serde_json::from_str(&raw).map_err(|_|"index_incomplete: Kiro context metadata is invalid")?;
    kiro_store::parse::context(&value["model"],&value["percent"])
}

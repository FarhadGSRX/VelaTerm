//! Info-panel queries over the conversation's verified native metadata.

use crate::{agent::kiro_store,db::repo,host::AppCtx,models::SessionKind};

pub fn kiro(app:&AppCtx,session:&str) -> Result<kiro_store::Context,String> {
    let session=repo::get_session(&app.db().conn.lock().unwrap(),session).map_err(|e|e.to_string())?.ok_or("Session not found")?;
    if session.kind!=SessionKind::Kiro {return Err("unsupported_record: Kiro context requires a Kiro session".into());}
    let id=session.agent_session_id.filter(|id|!id.is_empty()).ok_or("source_missing: Kiro history is unavailable")?;
    super::kiro::context(&id)
}

//! Native Antigravity events feed the shared timeline, queue and process lifecycle.

use super::*;

pub(super) fn handle_line(app: &AppCtx, session_id: &str, proc: &Arc<ChatProcess>, line: &str) {
    let Ok(frame) = serde_json::from_str::<Value>(line) else { return };
    match frame.get("event").and_then(Value::as_str) {
        Some("init") => {
            remember_session(app, session_id, proc, frame.get("conversation_id").and_then(Value::as_str));
            if let Some(model) = frame.pointer("/init/model").and_then(Value::as_str) {
                *proc.model.lock().unwrap() = Some(model.into());
            }
            if let Some(mode) = frame.pointer("/init/permission_mode").and_then(Value::as_str) {
                *proc.mode.lock().unwrap() = if mode == "always-proceed" { "bypassPermissions" } else { "default" }.into();
                proc.permission_confirmed.store(true, Ordering::Relaxed);
            }
            emit(app, session_id, json!({"type":"settingsChanged","mode":proc.mode.lock().unwrap().clone(),
                "model":proc.model.lock().unwrap().clone()}));
        }
        Some("step_update") => {
            let Some(step) = frame.get("step_update") else { return };
            remember_session(app, session_id, proc, step.get("conversation_id").and_then(Value::as_str));
            if let Some(mut row) = proc.antigravity.lock().unwrap().step(step, proc.model.lock().unwrap().clone()) {
                if let ChatRow::Assistant { at, .. } = &mut row { *at = Some(now_ms() as i64); }
                proc.timeline.lock().unwrap().upsert(row);
            }
        }
        Some("result") => {
            let Some(result) = frame.get("result") else { return };
            remember_session(app, session_id, proc, result.get("conversation_id").and_then(Value::as_str));
            let row = proc.antigravity.lock().unwrap().result(result, proc.model.lock().unwrap().clone());
            if let Some(row) = row { proc.timeline.lock().unwrap().upsert(row); }
            finish_rows(proc);
            if let Some(usage) = result.get("usage") {
                proc.extras.lock().unwrap().native_usage = Some(json!({"scope":"session",
                    "inputTokens":usage.get("input_tokens"),"outputTokens":usage.get("output_tokens"),
                    "thinkingTokens":usage.get("thinking_tokens"),"cacheReadInputTokens":usage.get("cache_read_tokens")}));
                emit_extras(app, session_id, proc);
            }
            // SIGINT ends the process, not just the turn. Keep its queue until a replacement owns it.
            if proc.turn.lock().unwrap().interrupted { return; }
            let status = result.get("status").and_then(Value::as_str).unwrap_or("ERROR");
            let outcome = if status == "SUCCESS" { "success" } else {
                result.get("error").and_then(Value::as_str).unwrap_or(status)
            };
            handle_turn_end(app, session_id, proc, outcome, None);
        }
        _ => {},
    }
}

fn remember_session(app: &AppCtx, session_id: &str, proc: &Arc<ChatProcess>, id: Option<&str>) {
    let Some(id) = id.filter(|id| !id.is_empty()) else { return };
    if !verify_native_identity(app,session_id,proc,id) { return; }
    let mut previous = proc.agent_session_id.lock().unwrap();
    if previous.as_deref() == Some(id) { return; }
    *previous = Some(id.into());
    drop(previous);
    let changed = crate::db::repo::set_agent_session_id(&app.db().conn.lock().unwrap(), session_id, id, SessionKind::Antigravity)
        .unwrap_or(false);
    if changed { app.emit(crate::host::TREE_CHANGED, ()); }
    emit(app, session_id, json!({"type":"session","agentSessionId":id,"model":proc.model.lock().unwrap().clone()}));
}

pub(super) fn restart_state(previous: Option<Arc<ChatProcess>>) -> (TurnQueue, Arc<AtomicBool>) {
    let Some(previous) = previous.filter(|p| !p.alive.load(Ordering::Relaxed)) else {
        return (TurnQueue::default(), Arc::new(AtomicBool::new(false)));
    };
    let mut turn = previous.turn.lock().unwrap();
    let release = if turn.interrupted { previous.release_when_idle.clone() }
        else { Arc::new(AtomicBool::new(false)) };
    (TurnQueue { waiting: std::mem::take(&mut turn.waiting), ..TurnQueue::default() }, release)
}

pub(super) fn finish_rows(proc: &Arc<ChatProcess>) {
    let mut timeline = proc.timeline.lock().unwrap();
    let start = timeline.rows.iter().rposition(|row| matches!(row, ChatRow::User { .. } | ChatRow::Shell { .. })).unwrap_or(0);
    let mut changed = Vec::new();
    for row in &timeline.rows[start..] {
        let mut row = row.clone();
        match &mut row {
            ChatRow::Assistant { streaming, .. } | ChatRow::Reasoning { streaming, .. } if *streaming => { *streaming = false; }
            ChatRow::Tool { status, .. } if *status == "running" => { *status = "canceled"; }
            _ => continue,
        }
        changed.push(row);
    }
    for row in changed { timeline.upsert(row); }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn antigravity_frames_persist_the_native_id_and_finish_one_answer() {
        let app = super::super::tests::ctx("agy-frames");
        let proc = super::super::tests::inert_process(SessionKind::Antigravity);
        {
            let conn = app.db().conn.lock().unwrap();
            conn.execute("INSERT INTO projects(id,name,root_path,created_at) VALUES ('p','test','/tmp',0)", []).unwrap();
            conn.execute("INSERT INTO sessions(id,project_id,name,kind,engine,created_at) VALUES ('s','p','Antigravity 1','antigravity','chat',0)", []).unwrap();
        }
        proc.turn.lock().unwrap().running = true;
        proc.turn.lock().unwrap().started_at = Some(now_ms().saturating_sub(1500));
        for frame in [json!({"event":"init","conversation_id":"native-agy","init":{"permission_mode":"request-review"}}),
            json!({"event":"step_update","step_update":{"step_index":1,"step_type":"agent_response","state":"ACTIVE","text_delta":"hello"}}),
            json!({"event":"step_update","step_update":{"step_index":1,"step_type":"agent_response","state":"DONE","text_delta":" world"}}),
            json!({"event":"result","result":{"conversation_id":"native-agy","status":"SUCCESS","response":"hello world","duration_seconds":1.5}})] {
            handle_line(&app, "s", &proc, &frame.to_string());
        }
        assert!(!proc.turn.lock().unwrap().running);
        assert!(proc.permission_confirmed.load(Ordering::Relaxed));
        let rows = &proc.timeline.lock().unwrap().rows;
        assert_eq!(rows.len(), 1);
        assert!(matches!(&rows[0], ChatRow::Assistant { text, streaming:false, duration_ms:Some(_), .. } if text == "hello world"));
        assert_eq!(crate::db::repo::get_session(&app.db().conn.lock().unwrap(), "s").unwrap().unwrap().agent_session_id.as_deref(), Some("native-agy"));
    }

    #[test]
    fn antigravity_interrupted_result_keeps_followups_for_the_next_process() {
        let app = super::super::tests::ctx("agy-interrupted");
        let proc = super::super::tests::inert_process(SessionKind::Antigravity);
        proc.timeline.lock().unwrap().upsert(super::super::super::antigravity_protocol::tool_row(
            "pending-tool".into(), "view_file", &json!({}), None, false, false));
        {
            let mut turn = proc.turn.lock().unwrap();
            turn.running = true; turn.interrupted = true;
            turn.waiting.push(QueuedMessage { id:"next".into(), text:"followup".into(), images:Vec::new(),original:None });
        }
        handle_line(&app, "s", &proc, &json!({"event":"result","result":{"status":"ERROR","error":"context canceled","response":""}}).to_string());
        assert_eq!(proc.turn.lock().unwrap().waiting.len(), 1);
        assert!(matches!(&proc.timeline.lock().unwrap().rows[0], ChatRow::Tool { status:"canceled", .. }));
    }

    #[test]
    fn antigravity_restart_transfers_the_queue_and_releases_after_a_closed_view() {
        let proc = super::super::tests::inert_process(SessionKind::Antigravity);
        proc.alive.store(false, Ordering::Relaxed);
        proc.release_when_idle.store(true, Ordering::Relaxed);
        {
            let mut turn = proc.turn.lock().unwrap();
            turn.interrupted = true;
            turn.waiting.push(QueuedMessage { id:"next".into(), text:"followup".into(), images:Vec::new(),original:None });
        }
        let (next, release) = restart_state(Some(proc.clone()));
        assert!(release.load(Ordering::Relaxed));
        proc.release_when_idle.store(false, Ordering::Relaxed);
        assert!(!release.load(Ordering::Relaxed));
        proc.release_when_idle.store(true, Ordering::Relaxed);
        assert!(release.load(Ordering::Relaxed));
        assert_eq!(next.waiting[0].text, "followup");
        assert!(!next.running);
        assert!(proc.turn.lock().unwrap().waiting.is_empty());
    }
}

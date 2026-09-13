//! Persistent planning and execution with independent task workflows over the existing chat engine.

use std::io::Read;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub mod menu;
pub mod split;
use super::chat::protocol::ChatImage;

use crate::{
    command_core as core,
    db::repo,
    host::AppCtx,
    models::{Session, SessionKind},
};

pub const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS plan_execute_runs (
 id TEXT PRIMARY KEY, owner_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
 planner_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
 executor_id TEXT REFERENCES sessions(id) ON DELETE SET NULL,
 config TEXT NOT NULL, task TEXT NOT NULL, state TEXT NOT NULL, round INTEGER NOT NULL DEFAULT 0,
 summary TEXT NOT NULL DEFAULT ''
);
CREATE TABLE IF NOT EXISTS plan_execute_messages (
 id TEXT PRIMARY KEY, run_id TEXT NOT NULL REFERENCES plan_execute_runs(id) ON DELETE CASCADE,
 sender_id TEXT NOT NULL, target_id TEXT NOT NULL, action TEXT NOT NULL, round INTEGER NOT NULL,
 fingerprint TEXT NOT NULL, wire TEXT NOT NULL, origin TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS plan_execute_attachments (
 message_id TEXT PRIMARY KEY REFERENCES plan_execute_messages(id) ON DELETE CASCADE,
 images TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS plan_execute_target ON plan_execute_messages(target_id);
CREATE TABLE IF NOT EXISTS plan_execute_menu_launches (
 run_id TEXT PRIMARY KEY REFERENCES plan_execute_runs(id) ON DELETE CASCADE,
 request TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS plan_execute_proposals (
 run_id TEXT PRIMARY KEY REFERENCES plan_execute_runs(id) ON DELETE CASCADE,
 message_id TEXT NOT NULL UNIQUE, fingerprint TEXT NOT NULL, tasks TEXT NOT NULL,
 state TEXT NOT NULL, confirmation TEXT, notice_id TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS plan_execute_tasks (
 parent_id TEXT NOT NULL REFERENCES plan_execute_runs(id) ON DELETE CASCADE,
 run_id TEXT PRIMARY KEY REFERENCES plan_execute_runs(id) ON DELETE CASCADE,
 position INTEGER NOT NULL, name TEXT NOT NULL, dispatch_id TEXT NOT NULL UNIQUE,
 launch_pending INTEGER NOT NULL DEFAULT 1
);
CREATE INDEX IF NOT EXISTS plan_execute_tasks_parent ON plan_execute_tasks(parent_id);
";

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RoleConfig {
    pub agent: Option<SessionKind>,
    pub model: Option<String>,
    pub effort: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Config {
    /// Missing on older workflows, whose executors always share the planner's directory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_mode: Option<WorktreeMode>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub split_tasks: bool,
    #[serde(default)]
    pub plan: RoleConfig,
    #[serde(default)]
    pub exec: RoleConfig,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum WorktreeMode {
    None,
    Shared,
    Each,
}

impl Config {
    fn worktree_mode(&self, legacy_worktree: bool) -> WorktreeMode {
        self.worktree_mode.unwrap_or(if legacy_worktree {
            WorktreeMode::Shared
        } else {
            WorktreeMode::None
        })
    }
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Run {
    pub id: String,
    pub owner_id: String,
    pub planner_id: String,
    pub executor_id: Option<String>,
    pub config: Config,
    pub task: String,
    pub state: String,
    pub round: u32,
    pub summary: String,
}

pub fn supported(kind: SessionKind) -> bool {
    matches!(
        kind,
        SessionKind::Claude
            | SessionKind::Codex
            | SessionKind::Opencode
            | SessionKind::Pi
            | SessionKind::Omp
    )
}

pub(super) fn operation_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn session(app: &AppCtx, id: &str) -> Result<Session, String> {
    repo::get_session(&app.db().conn.lock().unwrap(), id)?
        .filter(|s| s.archived_at.is_none())
        .ok_or_else(|| "The workflow session is missing or archived".into())
}

fn get(app: &AppCtx, id: &str) -> Result<Run, String> {
    app.db().conn.lock().unwrap().query_row(
        "SELECT id,owner_id,planner_id,executor_id,config,task,state,round,summary FROM plan_execute_runs WHERE id=?1", [id],
        |r| Ok((r.get::<_,String>(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get::<_,String>(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?)),
    ).map_err(|_| "Workflow not found".to_string()).and_then(|(id,owner_id,planner_id,executor_id,config,task,state,round,summary)| {
        Ok(Run { id,owner_id,planner_id,executor_id,config:serde_json::from_str(&config).map_err(|e|e.to_string())?,task,state,round,summary })
    })
}

fn brief(run: &Run) -> Value {
    json!({"id":run.id,"ownerId":run.owner_id,"plannerId":run.planner_id,
        "executorId":run.executor_id,"config":run.config,"state":run.state,"round":run.round,
        "summary":run.summary.chars().take(2000).collect::<String>(),
        "summaryTruncated":run.summary.chars().count()>2000})
}

fn normalize(app: &AppCtx, config: &RoleConfig, parent: &Session) -> Result<RoleConfig, String> {
    let kind = config.agent.unwrap_or(if supported(parent.kind) {
        parent.kind
    } else {
        SessionKind::Claude
    });
    if !supported(kind) {
        return Err("Planning and execution require chat-capable agents".into());
    }
    let inherited = if kind == parent.kind && (config.model.is_none() || config.effort.is_none()) {
        super::session_settings::resolve(app, parent)?
    } else {
        Default::default()
    };
    let model = config.model.clone().or(inherited.model);
    let effort = config.effort.clone().or(inherited.effort);
    Ok(RoleConfig {
        agent: Some(kind),
        model,
        effort,
    })
}

/// Drafts must remain editable even when a caller supplied an invalid identifier. Validate only when
/// starting, before any session or worktree is created.
fn validate_config(config: &Config) -> Result<(), String> {
    for (role, value) in [("Planner", &config.plan), ("Executor", &config.exec)] {
        let kind = value.agent.ok_or("Missing workflow agent")?;
        if !supported(kind) {
            return Err(format!(
                "{role}: Planning and execution require chat-capable agents"
            ));
        }
        super::launch_options::apply(kind, None, value.model.as_deref(), value.effort.as_deref())
            .map_err(|error| format!("{role}: {error}"))?;
    }
    Ok(())
}

fn message_images(app: &AppCtx, id: &str) -> Result<Vec<ChatImage>, String> {
    let stored: Option<String> = app
        .db()
        .conn
        .lock()
        .unwrap()
        .query_row(
            "SELECT images FROM plan_execute_attachments WHERE message_id=?1",
            [id],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    stored
        .map(|value| serde_json::from_str(&value).map_err(|e| e.to_string()))
        .unwrap_or_else(|| Ok(vec![]))
}

fn save_images(conn: &rusqlite::Connection, id: &str, images: &[ChatImage]) -> Result<(), String> {
    if !images.is_empty() {
        conn.execute(
            "INSERT INTO plan_execute_attachments(message_id,images) VALUES (?1,?2)",
            params![
                id,
                serde_json::to_string(images).map_err(|e| e.to_string())?
            ],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub fn defaults(app: &AppCtx, parent_id: &str, config: &Config) -> Result<Value, String> {
    let parent = session(app, parent_id)?;
    let plan = normalize(app, &config.plan, &parent)?;
    let mut exec = config.exec.clone();
    if exec.agent.is_none() {
        exec.agent = plan.agent;
    }
    let exec = normalize(app, &exec, &parent)?;
    Ok(json!(Config { plan, exec, ..config.clone() }))
}

fn child(
    app: &AppCtx,
    parent: &Session,
    name: &str,
    config: &RoleConfig,
    cwd: Option<&str>,
    worktree: Option<&str>,
    base: Option<&str>,
) -> Result<Session, String> {
    create_role(
        app,
        &parent.project_id,
        parent.group_id.as_deref(),
        Some(parent),
        name,
        config,
        cwd,
        worktree,
        base,
    )
}

#[allow(clippy::too_many_arguments)]
fn create_role(
    app: &AppCtx,
    project_id: &str,
    group_id: Option<&str>,
    parent: Option<&Session>,
    name: &str,
    config: &RoleConfig,
    cwd: Option<&str>,
    worktree: Option<&str>,
    base: Option<&str>,
) -> Result<Session, String> {
    let kind = config.agent.ok_or("Missing workflow agent")?;
    let args = super::launch_options::apply(
        kind,
        None,
        config.model.as_deref(),
        config.effort.as_deref(),
    )?;
    let created = core::create_session(
        app,
        project_id,
        group_id,
        name,
        kind,
        None,
        cwd,
        None,
        parent.map(|p| p.id.as_str()),
        worktree,
        args.as_deref(),
        parent
            .filter(|p| p.kind == kind)
            .and_then(|p| p.permission_mode.as_deref()),
        None,
        base,
        None,
        parent
            .filter(|p| p.kind == kind)
            .and_then(|p| p.agent_path.as_deref()),
        Some("chat"),
    )?;
    super::session_settings::persist(
        app,
        &created,
        &super::session_settings::Selection {
            model: super::session_settings::clean(config.model.as_deref()),
            effort: super::session_settings::clean(config.effort.as_deref()),
        },
    )?;
    Ok(created)
}

/// Called only after the ordinary spawn card has been confirmed (or explicitly skipped).
pub fn start(app: &AppCtx, request: &super::server::SpawnRequest) -> Result<Value, String> {
    let _guard = operation_lock().lock().unwrap();
    let parent = session(app, &request.parent_session_id)?;
    let requested = request
        .plan_execute
        .as_ref()
        .ok_or("Missing planning configuration")?;
    let id = request
        .request_id
        .clone()
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    uuid::Uuid::parse_str(&id).map_err(|_| "Invalid workflow request ID")?;
    if request.prompt.trim().is_empty() {
        return Err("A workflow needs a task".into());
    }
    let plan = normalize(app, &requested.plan, &parent)?;
    // The execution role follows the selected agent unless explicitly configured, never a stale CLI flag.
    let mut exec = requested.exec.clone();
    if exec.agent.is_none() {
        exec.agent = plan.agent;
    }
    let exec = normalize(app, &exec, &parent)?;
    let config = Config { plan, exec, ..requested.clone() };
    validate_config(&config)?;
    core::check_images(&request.images)?;
    if let Ok(existing) = get(app, &id) {
        if existing.owner_id != parent.id
            || existing.task != request.prompt
            || message_images(app, &format!("msg-{id}"))? != request.images
            || serde_json::to_value(&existing.config).ok() != serde_json::to_value(&config).ok()
        {
            return Err(
                "This request ID already has a different task or launch configuration".into(),
            );
        }
        let planner = session(app, &existing.planner_id)?;
        if existing.round == 0 && existing.state == "blocked" {
            bootstrap(app, &existing)?;
        }
        return Ok(json!({"run":brief(&get(app,&id)?),"planner":planner}));
    }
    let root = repo::get_project_root(&app.db().conn.lock().unwrap(), &parent.project_id)?;
    let cwd = request.cwd.clone().or(parent.cwd.clone()).or(root);
    let wt = if config.worktree_mode(request.worktree == Some(true)) != WorktreeMode::None {
        Some(crate::git::worktree_add_sibling(
            cwd.as_deref().ok_or("Select a working directory")?,
            "plan-execute",
        )?)
    } else {
        None
    };
    let cwd = wt.as_ref().map(|w| w.path.as_str()).or(cwd.as_deref());
    let planner = child(
        app,
        &parent,
        "Plan · Execute",
        &config.plan,
        cwd,
        wt.as_ref().map(|w| w.path.as_str()),
        wt.as_ref().map(|w| w.base_ref.as_str()),
    )?;
    let task = format!(
        "{}\n\nWorkflow ID: {id}\nRole: planner\nWorking directory: {}\n\nUser task:\n{}",
        include_str!("../../../skills/vspawn/references/plan-execute.md"),
        cwd.unwrap_or(""),
        request.prompt
    );
    let message_id = format!("msg-{id}");
    let origin = json!({"sessionId":parent.id,"name":parent.name,"agent":parent.kind,"role":"initiator","runId":id,"round":0});
    let wire = format!("[VelaTerm message {message_id}]\n{origin}\n\n{task}")
        .trim_end()
        .to_owned();
    {
        let mut conn = app.db().conn.lock().unwrap();
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        tx.execute("INSERT INTO plan_execute_runs(id,owner_id,planner_id,config,task,state) VALUES (?1,?2,?3,?4,?5,'planning')",
            params![id,parent.id,planner.id,serde_json::to_string(&config).map_err(|e|e.to_string())?,request.prompt]).map_err(|e|e.to_string())?;
        tx.execute("INSERT INTO plan_execute_messages(id,run_id,sender_id,target_id,action,round,fingerprint,wire,origin) VALUES (?1,?2,?3,?4,'start',0,'start',?5,?6)",
            params![message_id,id,parent.id,planner.id,wire,origin.to_string()]).map_err(|e|e.to_string())?;
        save_images(&tx, &message_id, &request.images)?;
        tx.commit().map_err(|e| e.to_string())?;
    }
    bootstrap(app, &get(app, &id)?)?;
    Ok(json!({"run":brief(&get(app,&id)?),"planner":planner}))
}

fn bootstrap(app: &AppCtx, run: &Run) -> Result<(), String> {
    let (id, wire): (String, String) = app
        .db()
        .conn
        .lock()
        .unwrap()
        .query_row(
            "SELECT id,wire FROM plan_execute_messages WHERE run_id=?1 AND action='start'",
            [&run.id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|e| e.to_string())?;
    // Retrying uses the original session and submission ID, including uncertain-delivery protection.
    let outcome = deliver(app, run, &run.planner_id, &wire, &id);
    let (state, summary) = match outcome {
        Ok(_) => ("planning", String::new()),
        Err(error) => ("blocked", error),
    };
    app.db()
        .conn
        .lock()
        .unwrap()
        .execute(
            "UPDATE plan_execute_runs SET state=?2,summary=?3 WHERE id=?1",
            params![run.id, state, summary],
        )
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Request {
    pub session_id: String,
    pub run_id: String,
    pub action: String,
    #[serde(default)]
    pub message_id: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub round: u32,
}

fn fingerprint(req: &Request) -> String {
    format!(
        "{:x}",
        Sha256::digest(
            serde_json::to_vec(&json!([req.session_id, req.action, req.round, req.text])).unwrap()
        )
    )
}

fn health(app: &AppCtx, id: &str) -> Value {
    let snapshot = app.chat().snapshot_window(id, Some(&Default::default()));
    let (_, states) = super::status_watch::snapshot();
    let current = session(app, id).ok();
    json!({"sessionId":id,"cwd":current.as_ref().and_then(|s|s.cwd.as_deref()),
        "worktreePath":current.as_ref().and_then(|s|s.worktree_path.as_deref()),
        "running":snapshot.running,"state":states.iter().find(|s|s.session_id==id).map(|s|s.state),
        "permissionPending":!snapshot.permissions.is_empty(),"queued":snapshot.queue.len(),
        "error":snapshot.rows.iter().rev().take_while(|r| !matches!(r, super::chat::engine::ChatRow::User{..})).find_map(|r| match r {super::chat::engine::ChatRow::Error{message,..}=>Some(message),_=>None})})
}

pub fn action(app: &AppCtx, req: &Request) -> Result<Value, String> {
    if req.action == "report" {
        return Err("Submit execution reports with vtell --report".into());
    }
    let _guard = operation_lock().lock().unwrap();
    apply_action(app, req)
}

/// Resolve a report from its real executor; a target override must name that workflow's planner.
pub fn report(
    app: &AppCtx,
    req: &super::tell::Request,
    target: Option<&str>,
) -> Result<Value, String> {
    let round = req
        .round
        .ok_or("Reports require the current --round N from vflow status")?;
    let _guard = operation_lock().lock().unwrap();
    let run_id = {
        let conn = app.db().conn.lock().unwrap();
        let saved: Option<String> = conn
            .query_row(
                "SELECT run_id FROM plan_execute_messages WHERE id=?1",
                [&req.message_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        if let Some(id) = saved {
            id
        } else {
            let mut query = conn.prepare("SELECT id FROM plan_execute_runs WHERE executor_id=?1 AND state NOT IN ('completed','stopped')").map_err(|e|e.to_string())?;
            let ids = query
                .query_map([&req.session_id], |r| r.get::<_, String>(0))
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?;
            if ids.len() != 1 {
                return Err("This session must be the executor of one active workflow".into());
            }
            ids[0].clone()
        }
    };
    let run = get(app, &run_id)?;
    if run.executor_id.as_deref() != Some(req.session_id.as_str()) {
        return Err("Only the workflow executor can submit a report".into());
    }
    if target.is_some_and(|id| id != run.planner_id) {
        return Err("A report must target this workflow's planning session".into());
    }
    apply_action(
        app,
        &Request {
            session_id: req.session_id.clone(),
            run_id,
            action: "report".into(),
            message_id: req.message_id.clone(),
            text: req.text.clone(),
            round,
        },
    )
}

fn apply_action(app: &AppCtx, req: &Request) -> Result<Value, String> {
    let mut run = get(app, &req.run_id)?;
    split::check_action(app, &run, &req.action)?;
    let is_plan = req.session_id == run.planner_id;
    let is_exec = run.executor_id.as_deref() == Some(req.session_id.as_str());
    if !is_plan && !is_exec && req.session_id != run.owner_id {
        return Err("This session does not belong to the workflow".into());
    }
    session(app, &req.session_id)?;
    if req.action == "status" {
        let conn = app.db().conn.lock().unwrap();
        let mut query=conn.prepare("SELECT m.id,m.target_id,m.action,m.round,s.outcome FROM plan_execute_messages m LEFT JOIN chat_submissions s ON s.session_id=m.target_id AND s.id=m.id WHERE m.run_id=?1 ORDER BY m.rowid DESC LIMIT 5").map_err(|e|e.to_string())?;
        let receipts=query.query_map([&run.id],|r|Ok(json!({"messageId":r.get::<_,String>(0)?,"targetSessionId":r.get::<_,String>(1)?,"action":r.get::<_,String>(2)?,"round":r.get::<_,u32>(3)?,"receipt":r.get::<_,Option<String>>(4)?})))
            .map_err(|e|e.to_string())?.collect::<Result<Vec<_>,_>>().map_err(|e|e.to_string())?;
        drop(query);
        drop(conn);
        return Ok(
            json!({"planner":health(app,&run.planner_id),"executor":run.executor_id.as_deref().map(|id|health(app,id)),"run":brief(&run),"recentDeliveries":receipts,
                "parentRunId":split::parent_id(app,&run.id)?,"tasks":split::tasks(app,&run.id)?,
                "proposal":if run.config.split_tasks {split::read(app,&run.id).ok()} else {None}}),
        );
    }
    if req.action == "propose" { return split::propose(app, &run, req); }
    if req.action == "stop" {
        if run.state == "completed" {
            return Err("This workflow has already completed".into());
        }
        app.db()
            .conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE plan_execute_runs SET state='stopped' WHERE id=?1",
                [&run.id],
            )
            .map_err(|e| e.to_string())?;
        let mut failures = Vec::new();
        let mut runs = vec![run.clone()];
        for id in split::task_ids(app,&run.id)? { runs.push(get(app,&id)?); }
        let mut peers = Vec::new();
        // Stopping one task must not interrupt the planner shared by its siblings.
        if split::parent_id(app,&run.id)?.is_none() { peers.push(run.planner_id.clone()); }
        for item in &runs {
            app.db().conn.lock().unwrap().execute("UPDATE plan_execute_runs SET state='stopped' WHERE id=?1 AND state!='completed'",[&item.id]).map_err(|e|e.to_string())?;
            if item.state != "completed" {
                if let Some(id) = &item.executor_id { peers.push(id.clone()); }
            }
        }
        app.db().conn.lock().unwrap().execute("UPDATE plan_execute_proposals SET state='stopped' WHERE run_id=?1 AND state='pending'",[&run.id]).map_err(|e|e.to_string())?;
        for id in &peers {
            for item in app
                .chat()
                .snapshot_window(id, Some(&Default::default()))
                .queue
            {
                let owned=app.db().conn.lock().unwrap().query_row(
                    "SELECT 1 FROM plan_execute_messages WHERE id=?1 AND (run_id=?2 OR run_id IN (SELECT run_id FROM plan_execute_tasks WHERE parent_id=?2)) AND target_id=?3",
                    params![item.id,run.id,id],|_|Ok(())).optional().map_err(|e|e.to_string())?.is_some();
                if owned {
                    if let Err(error) = core::chat_queue_remove(app, id, &item.id) {
                        failures.push(format!("{id}: {error}"));
                    }
                }
            }
            if app.chat().turn_in_progress(id) {
                if let Err(error) = core::chat_interrupt(app, id) {
                    failures.push(format!("{id}: {error}"));
                }
            }
        }
        app.emit("plan-execute://proposal",json!({"runId":run.id,"resolved":true}));
        return Ok(
            json!({"run":brief(&get(app,&run.id)?),"interruptErrors":failures,
            "note":"Further workflow dispatches are disabled; an interrupted agent may still be finishing its current operation."}),
        );
    }

    super::tell::validate_message(&req.message_id, &req.text)?;
    let ordinary = app
        .db()
        .conn
        .lock()
        .unwrap()
        .query_row(
            "SELECT 1 FROM session_tells WHERE id=?1",
            [&req.message_id],
            |_| Ok(()),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    if ordinary.is_some() {
        return Err("This message ID already belongs to an ordinary session message".into());
    }
    let saved: Option<(String,String,String)> = app.db().conn.lock().unwrap().query_row(
        "SELECT fingerprint,target_id,wire FROM plan_execute_messages WHERE id=?1 AND run_id=?2",
        params![req.message_id,run.id], |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)),
    ).optional().map_err(|e|e.to_string())?;
    if let Some((hash, target, wire)) = saved {
        if hash != fingerprint(req) {
            return Err("This message ID already belongs to a different request".into());
        }
        if run.state == "stopped" {
            return Err("This workflow was stopped; pending actions cannot be replayed".into());
        }
        if req.round < run.round {
            return Err("This message belongs to an earlier workflow round".into());
        }
        return deliver(app, &run, &target, &wire, &req.message_id);
    }
    if matches!(run.state.as_str(), "completed" | "stopped") {
        return Err("This workflow has already ended".into());
    }
    let (target, state, role) = match req.action.as_str() {
        "dispatch"
            if is_plan
                && matches!(run.state.as_str(), "planning" | "reviewing" | "blocked")
                && req.round == run.round + 1 =>
        {
            if run.executor_id.is_none() {
                let planner = session(app, &run.planner_id)?;
                let name = split::task_name(app, &run.id)?.unwrap_or_else(|| "Execute".into());
                let wt = if run.config.worktree_mode == Some(WorktreeMode::Each) {
                    Some(crate::git::worktree_add_sibling(
                        planner.cwd.as_deref().ok_or("Select a working directory")?,
                        &name,
                    )?)
                } else {
                    None
                };
                let executor = child(
                    app,
                    &planner,
                    &name,
                    &run.config.exec,
                    wt.as_ref().map(|w| w.path.as_str()).or(planner.cwd.as_deref()),
                    wt.as_ref().map(|w| w.path.as_str()),
                    wt.as_ref().map(|w| w.base_ref.as_str()),
                )?;
                run.executor_id = Some(executor.id);
            }
            (run.executor_id.clone().unwrap(), "executing", "plan")
        }
        // A blocker records an incomplete handoff; it must not prevent the assigned executor from
        // completing that handoff after an interruption or reporting a result that is ready for review.
        "report"
            if is_exec
                && matches!(run.state.as_str(), "executing" | "blocked")
                && req.round == run.round =>
        {
            (run.planner_id.clone(), "reviewing", "exec")
        }
        "accept" if is_plan && (run.state == "reviewing" || (run.config.split_tasks && run.state == "blocked")) && req.round == run.round => {
            (run.owner_id.clone(), "completed", "plan")
        }
        "block" if (is_plan || is_exec) && req.round == run.round => (
            if is_exec {
                run.planner_id.clone()
            } else {
                run.owner_id.clone()
            },
            "blocked",
            if is_exec { "exec" } else { "plan" },
        ),
        _ => {
            return Err(
                "The role, action or round does not match the current workflow state".into(),
            )
        }
    };
    let sender = session(app, &req.session_id)?;
    let origin = json!({"sessionId":sender.id,"name":sender.name,"agent":sender.kind,"role":role,"runId":run.id,"round":req.round});
    let text = if req.action == "dispatch" && run.round == 0 {
        format!(
            "{}\n\nWorkflow ID: {}\nRole: executor\nRound: {}\nWorking directory: {}\n\nImplementation task:\n{}",
            include_str!("../../../skills/vspawn/references/plan-execute.md"),
            run.id,
            req.round,
            session(app, &target)?.cwd.as_deref().unwrap_or(""),
            req.text
        )
    } else {
        req.text.clone()
    };
    let wire = format!(
        "[VelaTerm message {}]\n{}\n\n{}",
        req.message_id, origin, text
    )
    .trim_end()
    .to_owned();
    // The executor receives the user's original references once, together with its first assignment.
    let images = if req.action == "dispatch" && run.round == 0 {
        message_images(app, &format!("msg-{}", run.id))?
    } else {
        vec![]
    };
    {
        let mut conn = app.db().conn.lock().unwrap();
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        tx.execute("INSERT INTO plan_execute_messages(id,run_id,sender_id,target_id,action,round,fingerprint,wire,origin) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![req.message_id,run.id,req.session_id,target,req.action,req.round,fingerprint(req),wire,origin.to_string()]).map_err(|e|e.to_string())?;
        save_images(&tx, &req.message_id, &images)?;
        tx.execute(
            "UPDATE plan_execute_runs SET executor_id=?2,state=?3,round=?4,summary=?5 WHERE id=?1",
            params![run.id, run.executor_id, state, req.round, req.text],
        )
        .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
    }
    if let Some(parent) = split::parent_id(app,&run.id)? { split::refresh_parent(app,&parent)?; }
    run = get(app, &run.id)?;
    deliver(app, &run, &target, &wire, &req.message_id)
}

fn deliver(app: &AppCtx, run: &Run, target: &str, wire: &str, id: &str) -> Result<Value, String> {
    // A menu-launched workflow is owned by its planner. Final audits and failure notices stay in its
    // ledger and conversation output rather than prompting the planner to review its own conclusion.
    if run.owner_id == run.planner_id && target == run.planner_id {
        let (action, sender): (String, String) = app
            .db()
            .conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT action,sender_id FROM plan_execute_messages WHERE id=?1 AND run_id=?2",
                params![id, run.id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|e| e.to_string())?;
        if sender == run.planner_id && matches!(action.as_str(), "accept" | "block" | "notice") {
            return Ok(
                json!({"delivery":"recorded","messageId":id,"targetSessionId":target,"run":brief(run)}),
            );
        }
    }
    let target_session = session(app, target)?;
    // The initiating session can be a terminal. Its final report stays in the workflow and planner.
    if target_session.engine != "chat" {
        return Ok(
            json!({"delivery":"retained","reason":"The initiating session is not a chat session","run":brief(&run)}),
        );
    }
    let outcome =
        super::tell::deliver_with_images(app, target, wire, id, message_images(app, id)?)?;
    Ok(json!({"delivery":outcome,"messageId":id,"targetSessionId":target,"run":brief(&run)}))
}

/// Workflow peers remain available while the user opens another pane.
pub fn active(app: &AppCtx, id: &str) -> bool {
    app.db().conn.lock().unwrap().query_row(
        "SELECT 1 FROM plan_execute_runs WHERE (planner_id=?1 OR executor_id=?1) AND state NOT IN ('completed','stopped') LIMIT 1",
        [id],|_|Ok(())).optional().ok().flatten().is_some()
}

/// Observe turn boundaries, not a polling timer. A missing report is a blocker, never a pass.
pub fn observe(app: &AppCtx, id: &str, event: &Value) {
    let kind = event["type"].as_str().unwrap_or("");
    if !matches!(kind, "turnCompleted" | "turnInterrupted" | "exited") || event["released"] == true
    {
        return;
    }
    let app = app.clone();
    let id = id.to_owned();
    let interrupted = kind != "turnCompleted";
    std::thread::spawn(move || {
        let _guard = operation_lock().lock().unwrap();
        let ids: Vec<String> = {
            let db = app.db();
            let conn = db.conn.lock().unwrap();
            let mut query = match conn.prepare("SELECT id FROM plan_execute_runs WHERE (executor_id=?1 AND state='executing') OR (planner_id=?1 AND state IN ('planning','reviewing'))") {
                Ok(query) => query, Err(_) => return,
            };
            let rows = match query.query_map([&id], |r|r.get(0)) { Ok(rows) => rows, Err(_) => return };
            rows.filter_map(Result::ok).collect()
        };
        for run_id in ids {
            if let Ok(run) = get(&app, &run_id) { observe_run(&app, &id, &run, interrupted); }
        }
    });
}

fn observe_run(app: &AppCtx, id: &str, run: &Run, interrupted: bool) {
        if app.chat().turn_in_progress(&id) && !interrupted {
            return;
        }
        let snapshot = app.chat().snapshot_window(&id, Some(&Default::default()));
        // A report may have already queued the next round while the previous turn is ending.
        if !interrupted
            && snapshot
                .queue
                .iter()
                .any(|q| app.db().conn.lock().unwrap().query_row(
                    "SELECT 1 FROM plan_execute_messages WHERE run_id=?1 AND target_id=?2 AND round=?3 AND wire=?4",
                    params![run.id,id,run.round,q.text], |_| Ok(())).optional().ok().flatten().is_some())
        {
            return;
        }
        if run.executor_id.as_deref() == Some(&id) {
            let current=snapshot.rows.iter().filter_map(|row| match row {
                super::chat::engine::ChatRow::User { text, .. } => Some(text), _=>None,
            }).any(|wire|app.db().conn.lock().unwrap().query_row(
                "SELECT 1 FROM plan_execute_messages WHERE run_id=?1 AND target_id=?2 AND round=?3 AND wire=?4",
                params![run.id,id,run.round,wire],|_|Ok(())).optional().ok().flatten().is_some());
            if !current && !interrupted {
                return;
            }
        }
        let summary = if interrupted {
            "A workflow session stopped before submitting its result. Inspect its conversation before resuming."
        } else {
            "A workflow turn ended without a dispatch, report or acceptance. Inspect its conversation and resolve the missing handoff before resuming."
        };
        let _ = app.db().conn.lock().unwrap().execute(
            "UPDATE plan_execute_runs SET state='blocked',summary=?2 WHERE id=?1",
            params![run.id, summary],
        );
        crate::diagnostics::record(
            "WARN",
            "plan_execute_handoff",
            json!({"runId":run.id,"sessionId":id,"round":run.round,"status":"blocked"}),
        );
        // The platform notice is distinct from an executor's report and never implies acceptance.
        let target = if run.executor_id.as_deref() == Some(&id) {
            &run.planner_id
        } else {
            &run.owner_id
        };
        let message_id = format!("msg-{}", uuid::Uuid::new_v4());
        let origin = json!({"sessionId":id,"name":"VelaTerm","agent":"terminal","role":"system","runId":run.id,"round":run.round});
        let wire=format!("[VelaTerm message {message_id}]\n{origin}\n\nWorkflow {} is blocked: {summary}\nSession: {id}\nUse vflow status {} before resuming. Blocking does not prevent ordinary vtell messages. The executor can submit the current round with vtell --report without another dispatch. Use a new planner dispatch for a new assignment.",run.id,run.id);
        let inserted=app.db().conn.lock().unwrap().execute(
            "INSERT INTO plan_execute_messages(id,run_id,sender_id,target_id,action,round,fingerprint,wire,origin) VALUES (?1,?2,?3,?4,'notice',?5,'system',?6,?7)",
            params![message_id,run.id,id,target,run.round,wire,origin.to_string()]);
        if inserted.is_ok() {
            let _ = deliver(&app, &run, target, &wire, &message_id);
        }
}

pub fn handle(app: AppCtx, mut request: crate::diagnostics::HttpRequest, token: String) {
    let started = Instant::now();
    let request_id = request.id().to_owned();
    let authorized = request
        .headers()
        .iter()
        .any(|h| h.field.equiv("Authorization") && h.value.as_str() == format!("Bearer {token}"));
    let mut body = String::new();
    let result = if !authorized {
        Err((401, "Authentication required".to_owned()))
    } else if request.method() != &tiny_http::Method::Post {
        Err((405, "Use POST".to_owned()))
    } else if request
        .as_reader()
        .take(131073)
        .read_to_string(&mut body)
        .is_err()
        || body.len() > 131072
    {
        Err((413, "Request too large".to_owned()))
    } else {
        serde_json::from_str::<Request>(&body)
            .map_err(|e| e.to_string())
            .and_then(|req| action(&app, &req))
            .map_err(|e| (400, e))
    };
    let (status, value) = match result {
        Ok(result) => (200, json!({"result":result})),
        Err((code, error)) => (code, json!({"error":error})),
    };
    let _ = request.respond(
        tiny_http::Response::from_string(value.to_string())
            .with_status_code(status)
            .with_header(
                tiny_http::Header::from_bytes("Content-Type", "application/json").unwrap(),
            ),
    );
    crate::diagnostics::record(
        if status == 200 { "INFO" } else { "WARN" },
        "plan_execute",
        json!({"requestId":request_id,"statusCode":status,"durationMs":started.elapsed().as_millis() as u64}),
    );
}

pub fn run_cli(args: &[String]) -> ! {
    let result = (|| -> Result<Value, String> {
        let rest = &args[args.len().min(2)..];
        if rest.is_empty() || rest[0] == "--help" {
            return Ok(
                json!({"usage":"vflow status|stop|propose|dispatch|accept|block <run-id> [--round N --message-id msg-UUID] < message.txt","note":"propose reads a JSON object with tasks [{name,prompt,config:{agent,model,effort}}]. It requires user confirmation before execution. Other mutation text is read from stdin. Reuse the exact message ID, round and text when retrying."}),
            );
        }
        if !matches!(
            rest[0].as_str(),
            "status" | "stop" | "propose" | "dispatch" | "accept" | "block"
        ) {
            return Err(if rest[0] == "report" {
                "Use vtell --report --round N to submit an execution report"
            } else {
                "Unknown workflow action"
            }
            .into());
        }
        let mut req = Request {
            session_id: std::env::var("VLX_SESSION_ID")
                .map_err(|_| "Run inside a VelaTerm session")?,
            run_id: rest.get(1).ok_or("Missing workflow ID")?.clone(),
            action: rest[0].clone(),
            round: 0,
            message_id: String::new(),
            text: String::new(),
        };
        let mut i = 2;
        while i < rest.len() {
            let value = rest.get(i + 1).ok_or("Missing option value")?;
            match rest[i].as_str() {
                "--round" => req.round = value.parse().map_err(|_| "Invalid round")?,
                "--message-id" => req.message_id = value.clone(),
                _ => return Err("Unknown option".into()),
            }
            i += 2;
        }
        if !matches!(req.action.as_str(), "status" | "stop") {
            std::io::stdin()
                .take(65537)
                .read_to_string(&mut req.text)
                .map_err(|_| "Cannot read task text")?;
        }
        super::tell::post_local("plan-execute", &serde_json::to_value(req).unwrap())
    })();
    match result {
        Ok(value) => {
            println!("{}", serde_json::to_string_pretty(&value).unwrap());
            std::process::exit(0)
        }
        Err(error) => {
            eprintln!("vflow: {error}");
            std::process::exit(1)
        }
    }
}

//! Persistent scheduling: independent sessions run concurrently, each session has one active compiler.
use super::{now, runner};
use crate::host::AppCtx;
use rusqlite::{params, OptionalExtension, TransactionBehavior};
use serde_json::json;
use std::sync::atomic::Ordering;
use std::time::Duration;

pub(super) enum Next {
    Idle,
    Waiting,
    Job(String),
}

pub(super) fn claim(conn: &mut rusqlite::Connection) -> Result<Next, String> {
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    runner::recover(&tx)?;
    // A session's replacement waits for its previous process to stop without blocking other sessions.
    let next: Option<String> = tx.query_row(
        "SELECT j.id FROM memory_jobs j JOIN memory_sources s ON s.id=j.source_id
         WHERE j.status='queued' AND NOT EXISTS (
             SELECT 1 FROM memory_jobs active JOIN memory_sources origin ON origin.id=active.source_id
             WHERE active.status IN ('running','cancelling') AND origin.session_id=s.session_id
         ) ORDER BY j.rowid LIMIT 1",
        [], |r| r.get(0),
    ).optional().map_err(|e| e.to_string())?;
    let Some(id) = next else {
        let waiting: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM memory_jobs WHERE status='queued')",
                [],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        return Ok(if waiting { Next::Waiting } else { Next::Idle });
    };
    tx.execute(
        "UPDATE memory_jobs SET status='running',stage='extract',owner_pid=?2,updated_at=?3 WHERE id=?1",
        params![id, std::process::id(), now()],
    ).map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(Next::Job(id))
}

pub(super) fn resume(app: &AppCtx) -> Result<(), String> {
    // Admission and the worker's idle transition share this lock to avoid lost wakeups.
    let _conn = app.db().conn.lock().map_err(|e| e.to_string())?;
    if !_conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM memory_jobs WHERE status='queued')",
            [],
            |r| r.get::<_, bool>(0),
        )
        .map_err(|e| e.to_string())?
    {
        return Ok(());
    }
    if app.db().memory_worker.swap(true, Ordering::SeqCst) {
        return Ok(());
    }
    let ctx = app.clone();
    std::thread::Builder::new()
        .name("memory-scheduler".into())
        .spawn(move || loop {
            let next = match ctx.db().conn.lock() {
                Ok(mut conn) => {
                    let next = claim(&mut conn);
                    if matches!(next, Ok(Next::Idle)) {
                        ctx.db().memory_worker.store(false, Ordering::SeqCst);
                        return;
                    }
                    next
                }
                Err(error) => Err(error.to_string()),
            };
            match next {
                Ok(Next::Job(id)) => {
                    let worker_ctx = ctx.clone();
                    let worker_id = id.clone();
                    if std::thread::Builder::new()
                        .name("memory-compiler".into())
                        .spawn(move || runner::run_job(&worker_ctx, &worker_id))
                        .is_err()
                    {
                        runner::finish_job(&ctx, &id, Some("memory_interrupted".into()), 0);
                    }
                }
                Ok(Next::Waiting) => std::thread::sleep(Duration::from_millis(500)),
                Ok(Next::Idle) => unreachable!(),
                Err(error) => {
                    runner::process::audit(
                        &ctx,
                        "system",
                        "ERROR",
                        "queue_failed",
                        &json!({"error":error}),
                    );
                    std::thread::sleep(Duration::from_secs(2));
                }
            }
        })
        .map_err(|_| {
            app.db().memory_worker.store(false, Ordering::SeqCst);
            "memory_interrupted".to_string()
        })?;
    Ok(())
}

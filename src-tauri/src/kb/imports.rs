//! Import history: a durable row per import batch plus one row per candidate file.
//! Rows are audit data; deleting them never touches notes or attachments on disk.
use super::*;

/// A running import stops refreshing its heartbeat when its client dies; the next list read
/// retires it. Memory jobs use the same two-minute window.
const STALE_MS: i64 = 120_000;

pub(super) fn sweep(conn: &Connection) -> Result<()> {
    conn.execute(
        "UPDATE kb_imports SET status='interrupted',error='kb_interrupted',finished_at=?1,updated_at=?1 WHERE status='running' AND updated_at<?2",
        params![now(), now() - STALE_MS],
    )
    .map_err(err)?;
    Ok(())
}

/// Record a validated batch before its first chunk arrives. The caller removes the staging
/// directory when this fails, so a record without staged files cannot be left behind.
pub(super) fn create(
    conn: &Connection,
    vault_id: &str,
    import_id: &str,
    destination: &str,
    planned: &[(String, u64)],
    skipped: &[(String, u64)],
    bytes: u64,
) -> Result<()> {
    let tx = conn.unchecked_transaction().map_err(err)?;
    tx.execute(
        "INSERT INTO kb_imports(id,vault_id,destination,status,files,skipped,bytes,created_at,updated_at) VALUES(?1,?2,?3,'running',?4,?5,?6,?7,?7)",
        params![import_id, vault_id, destination, planned.len(), skipped.len(), bytes, now()],
    )
    .map_err(err)?;
    for (path, size) in planned {
        tx.execute(
            "INSERT INTO kb_import_files(import_id,path,size,status) VALUES(?1,?2,?3,'pending')",
            params![import_id, path, size],
        )
        .map_err(err)?;
    }
    for (path, size) in skipped {
        tx.execute(
            "INSERT INTO kb_import_files(import_id,path,size,status,reason) VALUES(?1,?2,?3,'skipped','hidden')",
            params![import_id, path, size],
        )
        .map_err(err)?;
    }
    tx.commit().map_err(err)
}

/// Refresh the heartbeat after a chunk lands. A heartbeat failure must not fail an upload whose
/// bytes are already staged, so callers ignore the result.
pub(super) fn touch(app: &AppCtx, vault_id: &str, import_id: &str) {
    if let Ok(conn) = app.db().conn.lock() {
        let _ = conn.execute(
            "UPDATE kb_imports SET updated_at=?3 WHERE id=?1 AND vault_id=?2 AND status='running'",
            params![import_id, vault_id, now()],
        );
    }
}

pub(super) fn complete(app: &AppCtx, vault_id: &str, import_id: &str, imported: usize) -> Result<()> {
    let conn = app.db().conn.lock().map_err(err)?;
    let tx = conn.unchecked_transaction().map_err(err)?;
    tx.execute(
        "UPDATE kb_imports SET status='completed',imported=?3,error='',finished_at=?4,updated_at=?4 WHERE id=?1 AND vault_id=?2",
        params![import_id, vault_id, imported, now()],
    )
    .map_err(err)?;
    tx.execute(
        "UPDATE kb_import_files SET status='imported' WHERE import_id=?1 AND status='pending'",
        [import_id],
    )
    .map_err(err)?;
    tx.commit().map_err(err)
}

/// Close a running batch as cancelled, or failed when the client reports an error code.
/// A batch already closed by commit is left untouched.
pub(super) fn finish(app: &AppCtx, vault_id: &str, import_id: &str, error: Option<&str>) -> Result<()> {
    let conn = app.db().conn.lock().map_err(err)?;
    conn.execute(
        "UPDATE kb_imports SET status=CASE WHEN ?3='' THEN 'cancelled' ELSE 'failed' END,error=?3,finished_at=?4,updated_at=?4 WHERE id=?1 AND vault_id=?2 AND status='running'",
        params![import_id, vault_id, error.unwrap_or(""), now()],
    )
    .map_err(err)?;
    Ok(())
}

pub fn list(app: &AppCtx, v: &Vault, args: &Value) -> Result<Value> {
    let page = args["page"].as_u64().unwrap_or(0);
    let conn = app.db().conn.lock().map_err(err)?;
    sweep(&conn)?;
    let total: u64 = conn
        .query_row("SELECT count(*) FROM kb_imports WHERE vault_id=?1", [&v.id], |r| r.get(0))
        .map_err(err)?;
    let mut stmt = conn
        .prepare("SELECT id,destination,status,error,files,imported,skipped,bytes,created_at,updated_at,finished_at FROM kb_imports WHERE vault_id=?1 ORDER BY created_at DESC,id DESC LIMIT ?2 OFFSET ?3")
        .map_err(err)?;
    let rows = stmt
        .query_map(params![v.id, PAGE, page * PAGE], |r| {
            Ok(json!({
                "id":r.get::<_,String>(0)?, "destination":r.get::<_,String>(1)?, "status":r.get::<_,String>(2)?,
                "error":r.get::<_,String>(3)?, "files":r.get::<_,i64>(4)?, "imported":r.get::<_,i64>(5)?,
                "skipped":r.get::<_,i64>(6)?, "bytes":r.get::<_,i64>(7)?, "createdAt":r.get::<_,i64>(8)?,
                "updatedAt":r.get::<_,i64>(9)?, "finishedAt":r.get::<_,Option<i64>>(10)?,
            }))
        })
        .map_err(err)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(err)?;
    Ok(json!({"imports":rows,"total":total,"pageSize":PAGE}))
}

pub fn detail(app: &AppCtx, v: &Vault, args: &Value) -> Result<Value> {
    let import_id = required(args, "importId")?;
    let conn = app.db().conn.lock().map_err(err)?;
    sweep(&conn)?;
    owned(&conn, &v.id, import_id)?;
    let mut stmt = conn
        .prepare("SELECT path,size,status,reason FROM kb_import_files WHERE import_id=?1 ORDER BY path")
        .map_err(err)?;
    let rows = stmt
        .query_map([import_id], |r| {
            Ok(json!({"path":r.get::<_,String>(0)?,"size":r.get::<_,i64>(1)?,"status":r.get::<_,String>(2)?,"reason":r.get::<_,String>(3)?}))
        })
        .map_err(err)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(err)?;
    Ok(json!({"files":rows}))
}

/// Remove one record and its file rows. Imported files stay on disk. A batch that is still
/// running cannot be deleted; an unfinished batch may leave a staging directory behind.
pub fn delete(app: &AppCtx, v: &Vault, args: &Value) -> Result<Value> {
    let import_id = required(args, "importId")?;
    let conn = app.db().conn.lock().map_err(err)?;
    sweep(&conn)?;
    let status: String = conn
        .query_row(
            "SELECT status FROM kb_imports WHERE id=?1 AND vault_id=?2",
            params![import_id, v.id],
            |r| r.get(0),
        )
        .map_err(|_| "kb_not_found")?;
    if status == "running" {
        return Err("kb_conflict".into());
    }
    conn.execute("DELETE FROM kb_imports WHERE id=?1", [import_id])
        .map_err(err)?;
    drop(conn);
    if status != "completed" {
        if let Ok(stage) = super::transfer::staging(v, import_id) {
            if stage.exists() {
                let _ = std::fs::remove_dir_all(stage);
            }
        }
    }
    Ok(json!(null))
}

const PAGE: u64 = 40;
fn owned(conn: &Connection, vault_id: &str, import_id: &str) -> Result<()> {
    let exists: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM kb_imports WHERE id=?1 AND vault_id=?2)",
            params![import_id, vault_id],
            |r| r.get(0),
        )
        .map_err(err)?;
    if exists {
        Ok(())
    } else {
        Err("kb_not_found".into())
    }
}

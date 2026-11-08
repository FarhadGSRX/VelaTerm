//! One-time removal of the search tables the business database used to hold.
//!
//! The index now lives in its own file ([`super::corpus`]). The old tables can hold about a gigabyte of
//! fragments, and dropping them in one statement keeps the write lock for seconds, so the large shadow
//! tables are emptied in short transactions first. A final `VACUUM` returns the space to the file system.
//! Runs on its own connection in a background thread; other writers wait on the busy timeout.

use rusqlite::Connection;

use crate::db::table_exists;
use crate::host::AppCtx;

const TABLES:[&str;3]=["session_words","session_fts","search_index_state"];
/// FTS5 shadow tables keyed by an integer `id`, which can be emptied piecewise.
const SHADOWS:[&str;6]=["session_words_data","session_words_content","session_words_docsize",
    "session_fts_data","session_fts_content","session_fts_docsize"];
const BATCH:usize=2000;

pub(super) fn remove(app:&AppCtx) {
    let started=std::time::Instant::now();
    match run(app) {
        Ok(false)=>{},
        Ok(true)=>{crate::diagnostics::record("INFO","legacy_search_removed",serde_json::json!({"durationMs":started.elapsed().as_millis() as u64}));},
        Err(_)=>{crate::diagnostics::record("WARN","legacy_search_removed",serde_json::json!({"status":"failed","errorCode":"operation_failed"}));},
    }
}

/// Whether there was anything to remove.
fn run(app:&AppCtx) -> Result<bool,String> {
    let path={
        let conn=app.db().conn.lock().unwrap();
        if !TABLES.iter().any(|table|table_exists(&conn,table)) {return Ok(false);}
        conn.path().map(str::to_owned).filter(|path|!path.is_empty()).ok_or("The application database has no file")?
    };
    let conn=Connection::open(&path).map_err(|e|e.to_string())?;
    conn.busy_timeout(std::time::Duration::from_secs(30)).map_err(|e|e.to_string())?;
    for shadow in SHADOWS {
        if !table_exists(&conn,shadow) {continue;}
        loop {
            // FTS5 keeps its averages and structure records at ids 1 and 10 of `_data`; without them
            // the table cannot be opened, and DROP TABLE needs to open it.
            let removed=conn.execute(&format!("DELETE FROM {shadow} WHERE id IN (SELECT id FROM {shadow} WHERE id>10 LIMIT {BATCH})"),[]).map_err(|e|e.to_string())?;
            if removed==0 {break;}
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
    conn.execute_batch("BEGIN IMMEDIATE").map_err(|e|e.to_string())?;
    let dropped=TABLES.iter().try_for_each(|table|conn.execute(&format!("DROP TABLE IF EXISTS {table}"),[]).map(|_|()));
    match dropped {Ok(())=>conn.execute_batch("COMMIT").map_err(|e|e.to_string())?,Err(error)=>{let _=conn.execute_batch("ROLLBACK");return Err(error.to_string());}}
    // The file shrinks only after a VACUUM; an active writer elsewhere just delays it.
    for _ in 0..10 {
        match conn.execute_batch("VACUUM") {Ok(())=>break,Err(_)=>std::thread::sleep(std::time::Duration::from_secs(3))}
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn old_search_tables_are_removed_and_the_file_shrinks() {
        let path=std::env::temp_dir().join(format!("vlx-search-legacy-{}",uuid::Uuid::new_v4()));std::fs::create_dir_all(&path).unwrap();
        let file=path.join("business.sqlite");
        {
            let conn=Connection::open(&file).unwrap();
            conn.execute_batch("PRAGMA journal_mode=WAL;
                CREATE VIRTUAL TABLE session_fts USING fts5(text,session_id UNINDEXED,tokenize='trigram remove_diacritics 1');
                CREATE VIRTUAL TABLE session_words USING fts5(words,session_id UNINDEXED,tokenize='porter unicode61 remove_diacritics 1');
                CREATE TABLE search_index_state(session_id TEXT,source TEXT);").unwrap();
            let text="deploy the worktree branch and compare every fragment ".repeat(40);
            conn.execute_batch("BEGIN").unwrap();
            for _ in 0..3000 {
                conn.execute("INSERT INTO session_fts(text,session_id) VALUES(?1,'s')",[&text]).unwrap();
                conn.execute("INSERT INTO session_words(rowid,words,session_id) VALUES(last_insert_rowid(),?1,'s')",[&text]).unwrap();
            }
            conn.execute_batch("COMMIT; PRAGMA wal_checkpoint(TRUNCATE);").unwrap();
        }
        let before=std::fs::metadata(&file).unwrap().len();
        let db=crate::db::Db::open(&file).unwrap();
        let app=AppCtx::Headless(Arc::new(crate::host::HeadlessHost::new(path.clone(),db)));
        assert!(run(&app).unwrap());
        {
            let conn=app.db().conn.lock().unwrap();
            for table in TABLES {assert!(!table_exists(&conn,table),"{table}");}
            conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);").unwrap();
            // The business database still works after the cleanup.
            assert_eq!(conn.query_row("SELECT count(*) FROM sessions",[],|row|row.get::<_,i64>(0)).unwrap(),0);
        }
        assert!(std::fs::metadata(&file).unwrap().len()*4<before,"the file shrinks after the cleanup");
        assert!(!run(&app).unwrap(),"a second run has nothing left to remove");
        drop(app);let _=std::fs::remove_dir_all(&path);
    }
}

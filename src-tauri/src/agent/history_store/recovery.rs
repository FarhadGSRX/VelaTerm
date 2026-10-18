//! Only derived databases may be replaced. Independently owned live originals survive rebuilding.

use std::io::{Read,Write};
use std::path::{Path,PathBuf};
use rusqlite::Connection;

fn initialize(path:&Path) -> rusqlite::Result<Connection> {
    let conn=Connection::open(path)?;
    conn.busy_timeout(std::time::Duration::from_secs(2))?;
    conn.execute_batch("PRAGMA auto_vacuum=INCREMENTAL;")?;
    conn.execute_batch(include_str!("schema.sql"))?;Ok(conn)
}

fn retain_shared(lease:&std::fs::File) -> Result<(),String> {
    // LockFileEx adds a second lock instead of downgrading an existing exclusive lock.
    #[cfg(windows)] lease.unlock().map_err(|_|"index_busy: history index ownership cannot be changed")?;
    lease.lock_shared().map_err(|_|"index_busy: history index ownership cannot be retained".into())
}

pub(super) fn open(path:&Path,data_dir:&Path) -> Result<(Connection,std::fs::File),String> {
    let mut options=std::fs::OpenOptions::new();options.create(true).truncate(false).read(true).write(true);
    #[cfg(unix)] {use std::os::unix::fs::OpenOptionsExt;options.mode(0o600);}
    let lease=options.open(path.parent().unwrap().join("index.lock")).map_err(|_|"index_busy: history index ownership is unavailable")?;
    lease.try_lock_shared().map_err(|_|"index_busy: history index is being replaced by another process")?;
    let upgrade=||->Result<(),String> {
        lease.unlock().map_err(|_|"index_busy: history index ownership cannot be changed")?;
        lease.try_lock().map_err(|_|"index_busy: another process is still reading this history index")?;Ok(())
    };
    if journal(path).exists() {upgrade()?;recover_switch(path)?;}
    let existed=path.exists();
    if existed {
        let version=Connection::open_with_flags(path,rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).and_then(|conn| {
            let version=conn.query_row("SELECT version FROM index_meta",[],|row|row.get::<_,u32>(0))?;
            let vacuum=conn.query_row("PRAGMA auto_vacuum",[],|row|row.get::<_,u32>(0))?;Ok((version,vacuum))
        });
        if let Ok((version,vacuum))=version {
            if version>super::limits::SCHEMA_VERSION {return Err("History index schema requires a supported migration".into());}
            if version<super::limits::SCHEMA_VERSION || vacuum!=2 {
                upgrade()?;let conn=replace(path,data_dir)?;retain_shared(&lease)?;return Ok((conn,lease));
            }
        } else {upgrade()?;}
    } else {upgrade()?;}
    let result=match initialize(path) {
        Ok(conn)=>{if !existed {restore_original_ledger(&conn,data_dir)?;} Ok(conn)},
        Err(rusqlite::Error::SqliteFailure(error,_)) if matches!(error.code,rusqlite::ErrorCode::DatabaseCorrupt|rusqlite::ErrorCode::NotADatabase)=>{
            upgrade()?;replace(path,data_dir)
        },
        Err(error)=>Err(super::store::sql(error)),
    }?;
    retain_shared(&lease)?;Ok((result,lease))
}

#[derive(serde::Serialize,serde::Deserialize)]
struct Switch {version:u32,token:String}
fn sibling(path:&Path,token:&str,phase:&str) -> PathBuf {path.parent().unwrap().join(format!("index.{phase}-{token}.sqlite"))}
fn journal(path:&Path) -> PathBuf {path.parent().unwrap().join("index-switch.json")}
fn synchronize_directory(path:&Path) -> Result<(),String> {
    #[cfg(unix)] {std::fs::File::open(path.parent().unwrap()).and_then(|file|file.sync_all()).map_err(|_|"storage_pressure: cannot persist the history index switch")?;}
    Ok(())
}
fn remove_derived(path:&Path) -> Result<(),String> {
    for ending in ["","-wal","-shm"] {
        let target=PathBuf::from(format!("{}{ending}",path.to_string_lossy()));
        match std::fs::remove_file(target) {Ok(())=>{},Err(error) if error.kind()==std::io::ErrorKind::NotFound=>{},Err(_)=>return Err("History index recovery cannot remove obsolete derived data".into())}
    }Ok(())
}
fn move_derived(from:&Path,to:&Path) -> Result<(),String> {
    // The main file moves last, so a missing main path proves its old sidecars were isolated.
    for ending in ["-wal","-shm",""] {
        let source=PathBuf::from(format!("{}{ending}",from.to_string_lossy()));let destination=PathBuf::from(format!("{}{ending}",to.to_string_lossy()));
        if source.exists() {std::fs::rename(source,destination).map_err(|_|"History index recovery cannot switch the derived database")?;}
    }Ok(())
}
fn ready(path:&Path) -> Result<(),String> {
    let conn=Connection::open_with_flags(path,rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(super::store::sql)?;
    let check:String=conn.query_row("PRAGMA quick_check(1)",[],|row|row.get(0)).map_err(super::store::sql)?;
    let version:u32=conn.query_row("SELECT version FROM index_meta",[],|row|row.get(0)).map_err(super::store::sql)?;
    if check!="ok" || version!=super::limits::SCHEMA_VERSION {return Err("History index replacement did not pass verification".into());}Ok(())
}
fn recover_switch(path:&Path) -> Result<(),String> {
    let state=journal(path);let mut bytes=Vec::new();
    let file=match std::fs::File::open(&state) {Ok(file)=>file,Err(error) if error.kind()==std::io::ErrorKind::NotFound=>return Ok(()),Err(_)=>return Err("History index switch journal is unavailable".into())};
    file.take(64*1024+1).read_to_end(&mut bytes).map_err(|_|"History index switch journal cannot be read")?;
    if bytes.len()>64*1024 {return Err("History index switch journal exceeds its metadata budget".into());}
    let switch:Switch=serde_json::from_slice(&bytes).map_err(|_|"History index switch journal is invalid")?;
    if switch.version!=1 || uuid::Uuid::parse_str(&switch.token).is_err() {return Err("History index switch journal is invalid".into());}
    let next=sibling(path,&switch.token,"next");let previous=sibling(path,&switch.token,"previous");
    if path.exists() && next.exists() {
        // An interrupted sidecar move has not yet removed the old main file. Restore those sidecars
        // before abandoning the prepared replacement, rather than mixing old and new databases.
        move_derived(&previous,path)?;
    }
    if !path.exists() {
        if next.exists() && ready(&next).is_ok() {std::fs::rename(&next,path).map_err(|_|"History index recovery cannot activate its prepared database")?;}
        else if previous.exists() {move_derived(&previous,path)?;}
        else {return Err("History index recovery has no complete derived database to restore".into());}
    } else if !next.exists() && previous.exists() && ready(path).is_err() {
        remove_derived(path)?;move_derived(&previous,path)?;
    }
    remove_derived(&next)?;remove_derived(&previous)?;
    std::fs::remove_file(state).map_err(|_|"History index recovery cannot clear its switch journal")?;synchronize_directory(path)
}
fn replace(path:&Path,data_dir:&Path) -> Result<Connection,String> {
    let token=uuid::Uuid::new_v4().to_string();let next=sibling(path,&token,"next");
    let prepared=(|| {
        let conn=initialize(&next).map_err(super::store::sql)?;restore_original_ledger(&conn,data_dir)?;
        conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)").map_err(super::store::sql)?;
        conn.close().map_err(|(_,error)|super::store::sql(error))?;ready(&next)?;
        std::fs::OpenOptions::new().read(true).write(true).open(&next).and_then(|file|file.sync_all())
            .map_err(|_|"storage_pressure: cannot persist the prepared history index")?;Ok(())
    })();
    if let Err(error)=prepared {let _=remove_derived(&next);return Err(error);}
    let state=journal(path);let mut options=std::fs::OpenOptions::new();options.write(true).create_new(true);
    #[cfg(unix)] {use std::os::unix::fs::OpenOptionsExt;options.mode(0o600);}
    let mut file=match options.open(&state) {Ok(file)=>file,Err(_)=>{let _=remove_derived(&next);return Err("History index recovery cannot create its switch journal".into());}};
    let persisted=file.write_all(&serde_json::to_vec(&Switch {version:1,token:token.clone()}).map_err(|e|e.to_string())?).and_then(|_|file.sync_all());drop(file);
    if persisted.is_err() {let _=std::fs::remove_file(&state);let _=remove_derived(&next);return Err("storage_pressure: cannot persist the history index switch journal".into());}
    synchronize_directory(path)?;
    let previous=sibling(path,&token,"previous");move_derived(path,&previous)?;
    recover_switch(path)?;
    crate::diagnostic_warn!("history index rebuilt; unique live originals retained");initialize(path).map_err(super::store::sql)
}

fn restore_original_ledger(conn:&Connection,data_dir:&Path) -> Result<(),String> {
    let root=data_dir.join("chat-spool");
    let entries=match std::fs::read_dir(&root) {Ok(entries)=>entries,Err(error) if error.kind()==std::io::ErrorKind::NotFound=>return Ok(()),
        Err(_)=>return Err("History recovery cannot inspect unique live originals".into())};
    conn.execute_batch("BEGIN IMMEDIATE").map_err(super::store::sql)?;
    let result=(|| {
        let mut count=0;
        for entry in entries {
            let entry=entry.map_err(|_|"History recovery cannot inspect a live original")?;
            let name=entry.file_name();let Some(name)=name.to_str() else {continue;};
            let Some(reference)=name.strip_suffix(".meta.json") else {continue;};
            if uuid::Uuid::parse_str(reference).is_err() {continue;}
            let metadata=std::fs::symlink_metadata(entry.path()).map_err(|_|"History recovery cannot inspect original ownership")?;
            if !metadata.is_file() || metadata.len()>64*1024 {continue;}
            let mut bytes=Vec::with_capacity(metadata.len() as usize);
            std::fs::File::open(entry.path()).map_err(|_|"History recovery cannot read original ownership")?.take(64*1024+1)
                .read_to_end(&mut bytes).map_err(|_|"History recovery cannot read original ownership")?;
            if bytes.len()>64*1024 {continue;}
            let header:serde_json::Value=match serde_json::from_slice(&bytes) {Ok(value)=>value,Err(_)=>continue};
            let Some(scope)=header["sessionId"].as_str().filter(|scope|!scope.is_empty() && scope.len()<=200) else {continue;};
            if header["version"]!=1 || header["reference"].as_str()!=Some(reference) || header["processEpoch"].as_u64().is_none() {continue;}
            let path=root.join(format!("{reference}.jsonl"));
            let metadata=match std::fs::symlink_metadata(&path) {Ok(metadata) if metadata.is_file()=>metadata,_=>continue};
            conn.execute("INSERT OR IGNORE INTO spool_entries(ref,session_scope,path,bytes,reconciled,reason) VALUES(?1,?2,?3,?4,0,'recovered_unique_original')",
                rusqlite::params![reference,scope,path.to_string_lossy(),metadata.len()]).map_err(super::store::sql)?;
            count+=1;
            if count%128==0 {conn.execute_batch("COMMIT; BEGIN IMMEDIATE").map_err(super::store::sql)?;}
        }
        Ok(())
    })();
    if result.is_err() {let _=conn.execute_batch("ROLLBACK");} else {conn.execute_batch("COMMIT").map_err(super::store::sql)?;}
    result
}

#[cfg(test)]
mod tests {
    #[test]
    fn interrupted_index_switch_recovers_before_opening_any_history_reader() {
        let temp=super::super::tests::Temp::new();let directory=temp.0.join("chat-index");std::fs::create_dir_all(&directory).unwrap();let path=directory.join("index.sqlite");
        let conn=super::initialize(&path).unwrap();conn.execute("INSERT INTO cursor_keys VALUES(1,'previous')",[]).unwrap();drop(conn);
        let token=uuid::Uuid::new_v4().to_string();let next=super::sibling(&path,&token,"next");let previous=super::sibling(&path,&token,"previous");
        let conn=super::initialize(&next).unwrap();conn.execute("INSERT INTO cursor_keys VALUES(1,'prepared')",[]).unwrap();drop(conn);
        std::fs::write(super::journal(&path),serde_json::to_vec(&super::Switch {version:1,token}).unwrap()).unwrap();
        super::move_derived(&path,&previous).unwrap();assert!(!path.exists());
        let (conn,_lease)=super::open(&path,&temp.0).unwrap();assert_eq!(conn.query_row("SELECT secret FROM cursor_keys",[],|row|row.get::<_,String>(0)).unwrap(),"prepared");
        assert!(!next.exists() && !previous.exists() && !super::journal(&path).exists());
    }

    #[test]
    fn old_schema_is_rebuilt_in_a_verified_database_while_unique_originals_survive() {
        let temp=super::super::tests::Temp::new();let directory=temp.0.join("chat-index");std::fs::create_dir_all(&directory).unwrap();let path=directory.join("index.sqlite");
        let conn=rusqlite::Connection::open(&path).unwrap();conn.execute_batch("CREATE TABLE index_meta(version INTEGER NOT NULL);INSERT INTO index_meta VALUES(1);").unwrap();drop(conn);
        let root=temp.0.join("chat-spool");std::fs::create_dir_all(&root).unwrap();let reference=uuid::Uuid::new_v4().to_string();let original=root.join(format!("{reference}.jsonl"));
        std::fs::write(&original,b"Only original").unwrap();std::fs::write(root.join(format!("{reference}.meta.json")),serde_json::json!({"version":1,"reference":reference,"sessionId":"owner","processEpoch":7}).to_string()).unwrap();
        let (conn,_lease)=super::open(&path,&temp.0).unwrap();
        assert_eq!(conn.query_row("SELECT version FROM index_meta",[],|row|row.get::<_,u32>(0)).unwrap(),super::super::limits::SCHEMA_VERSION);
        assert_eq!(conn.query_row("PRAGMA auto_vacuum",[],|row|row.get::<_,u32>(0)).unwrap(),2);
        assert_eq!(conn.query_row("SELECT count(*) FROM spool_entries WHERE ref=?1",[reference],|row|row.get::<_,u32>(0)).unwrap(),1);
        assert_eq!(std::fs::read(original).unwrap(),b"Only original");
        assert!(!std::fs::read_dir(directory).unwrap().any(|entry|{let name=entry.unwrap().file_name();let name=name.to_string_lossy();name.contains(".next-") || name.contains(".previous-") || name=="index-switch.json"}));
    }

    #[test]
    fn damaged_derived_database_rebuilds_without_deleting_unique_originals() {
        let temp=super::super::tests::Temp::new();let root=temp.0.join("chat-spool");std::fs::create_dir_all(&root).unwrap();
        let reference=uuid::Uuid::new_v4().to_string();let original=root.join(format!("{reference}.jsonl"));
        let bytes=b"{\"content\":\"Only copy\"}\n";std::fs::write(&original,bytes).unwrap();
        std::fs::write(root.join(format!("{reference}.meta.json")),serde_json::json!({"version":1,"reference":reference,"sessionId":"owner","processEpoch":7}).to_string()).unwrap();
        let directory=temp.0.join("chat-index");std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join("index.sqlite"),b"This is damaged derived data").unwrap();
        let store=super::super::store::Store::at(&temp.0).unwrap();
        let row:(String,u64,String)=store.connect().unwrap().query_row("SELECT session_scope,bytes,reason FROM spool_entries WHERE ref=?1",[reference],
            |row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).unwrap();
        assert_eq!(row,("owner".into(),bytes.len() as u64,"recovered_unique_original".into()));
        assert_eq!(std::fs::read(original).unwrap(),bytes);
        assert!(!std::fs::read_dir(directory).unwrap().any(|entry|entry.unwrap().file_name().to_string_lossy().contains(".rebuild-")));
    }

    #[test]
    fn independent_readers_share_the_index_but_prevent_a_version_switch_until_released() {
        let temp=super::super::tests::Temp::new();let directory=temp.0.join("chat-index");std::fs::create_dir_all(&directory).unwrap();let path=directory.join("index.sqlite");
        let (first,first_lease)=super::open(&path,&temp.0).unwrap();let (second,second_lease)=super::open(&path,&temp.0).unwrap();
        first.execute("UPDATE index_meta SET version=2",[]).unwrap();drop(first);drop(first_lease);
        assert!(super::open(&path,&temp.0).err().unwrap().starts_with("index_busy:"));
        assert_eq!(second.query_row("SELECT version FROM index_meta",[],|row|row.get::<_,u64>(0)).unwrap(),2);assert!(!super::journal(&path).exists());
        drop(second);drop(second_lease);
        let (current,_lease)=super::open(&path,&temp.0).unwrap();assert_eq!(current.query_row("SELECT version FROM index_meta",[],|row|row.get::<_,u32>(0)).unwrap(),super::super::limits::SCHEMA_VERSION);
    }

    #[test]
    fn a_switch_journal_creation_failure_removes_only_its_unused_prepared_database() {
        let temp=super::super::tests::Temp::new();let directory=temp.0.join("chat-index");std::fs::create_dir_all(&directory).unwrap();let path=directory.join("index.sqlite");
        let conn=super::initialize(&path).unwrap();conn.execute("INSERT INTO cursor_keys VALUES(1,'existing')",[]).unwrap();conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)").unwrap();drop(conn);
        let previous=std::fs::read(&path).unwrap();std::fs::create_dir(super::journal(&path)).unwrap();
        assert!(super::replace(&path,&temp.0).is_err());assert_eq!(std::fs::read(&path).unwrap(),previous);
        assert!(!std::fs::read_dir(directory).unwrap().any(|entry|entry.unwrap().file_name().to_string_lossy().contains(".next-")));
    }
}

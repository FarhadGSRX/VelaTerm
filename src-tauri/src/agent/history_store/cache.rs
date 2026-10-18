//! Reconstructible generation eviction. Sources and unique protocol originals are never removed.

use std::collections::{HashMap,HashSet};
use std::sync::{Arc,Mutex};
use std::time::{Duration,SystemTime,UNIX_EPOCH};
use rusqlite::{Connection,params};
use super::store::{Store,sql};

pub(super) const WAL_RESERVE:u64=32*1024*1024;
pub(super) fn budget() -> u64 {
    std::env::var("VLX_HISTORY_CACHE_BYTES").ok().and_then(|value|value.parse().ok())
        .filter(|value|*value>=64*1024*1024).unwrap_or(1024*1024*1024)
}
pub(super) fn now() -> i64 {SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis().min(i64::MAX as u128) as i64}

pub(super) fn scratch_files(data_dir:&std::path::Path) -> Result<HashMap<std::path::PathBuf,u64>,String> {
    let mut files=native_snapshots(data_dir)?;
    let entries=match std::fs::read_dir(data_dir.join("chat-index/tasks")) {Ok(entries)=>entries,
        Err(error) if error.kind()==std::io::ErrorKind::NotFound=>return Ok(files),Err(_)=>return Err("storage_pressure: history task storage is unavailable".into())};
    for entry in entries {
        let entry=entry.map_err(|_|"storage_pressure: history task storage is unavailable")?;let path=entry.path();
        if entry.file_name().to_str().and_then(|name|name.strip_prefix("owned-")).is_some_and(|id|uuid::Uuid::parse_str(id).is_ok()) {
            if !std::fs::symlink_metadata(&path).is_ok_and(|metadata|metadata.is_dir()) {continue;}
            let lease=path.join("owner.lock");
            if !std::fs::symlink_metadata(&lease).is_ok_and(|metadata|metadata.is_file()) {continue;}
            let mut owner=std::fs::OpenOptions::new().read(true).write(true).open(&lease).map_err(|_|"storage_pressure: history task ownership is unavailable")?;
            if owner.try_lock().is_ok() {drop(owner);std::fs::remove_dir_all(&path).map_err(|_|"storage_pressure: cannot remove abandoned history task data")?;continue;}
            use std::io::Read;let mut reservation=[0;8];owner.read_exact(&mut reservation).map_err(|_|"storage_pressure: history task reservation is invalid")?;
            let mut bytes=0u64;for item in std::fs::read_dir(&path).map_err(|_|"storage_pressure: history task data is unavailable")? {
                let item=item.map_err(|_|"storage_pressure: history task data is unavailable")?;
                let metadata=std::fs::symlink_metadata(item.path()).map_err(|_|"storage_pressure: history task data is unavailable")?;
                if metadata.is_file() {bytes=bytes.saturating_add(metadata.len());} else {return Err("storage_pressure: unexpected history task data".into());}
            }
            files.insert(path,bytes.max(u64::from_le_bytes(reservation)));continue;
        }
        if path.extension().and_then(|value|value.to_str())!=Some("summary") || path.file_stem().and_then(|value|value.to_str()).is_none_or(|value|uuid::Uuid::parse_str(value).is_err()) {continue;}
        let metadata=std::fs::symlink_metadata(&path).map_err(|_|"storage_pressure: cannot account for history task data")?;
        if !metadata.is_file() {continue;}
        if files.len()>=4096 {return Err("storage_pressure: too many retained history task files".into());}
        files.insert(path,metadata.len());
    }Ok(files)
}

fn native_snapshots(data_dir:&std::path::Path) -> Result<HashMap<std::path::PathBuf,u64>,String> {
    let entries=match std::fs::read_dir(data_dir.join("chat-index/native-snapshots")) {Ok(entries)=>entries,
        Err(error) if error.kind()==std::io::ErrorKind::NotFound=>return Ok(HashMap::new()),Err(_)=>return Err("storage_pressure: native snapshot storage is unavailable".into())};
    let mut files=HashMap::new();let mut seen=0;
    for entry in entries {
        let entry=entry.map_err(|_|"storage_pressure: native snapshot storage is unavailable")?;let path=entry.path();
        if entry.file_name().to_str().and_then(|name|name.strip_prefix("velaterm-kiro-")).is_none_or(|id|uuid::Uuid::parse_str(id).is_err()) {continue;}
        if !std::fs::symlink_metadata(&path).is_ok_and(|metadata|metadata.is_dir()) {continue;}
        seen+=1;if seen>64 {return Err("storage_pressure: too many retained native snapshots".into());}
        let lock=path.join("owner.lock");
        let mut reserved=0;
        if std::fs::symlink_metadata(&lock).is_ok_and(|metadata|metadata.is_file()) {
            let owner=std::fs::OpenOptions::new().read(true).write(true).open(&lock).map_err(|_|"storage_pressure: native snapshot ownership is unavailable")?;
            if owner.try_lock().is_ok() {
                drop(owner);
                std::fs::remove_dir_all(&path).map_err(|_|"storage_pressure: cannot remove an abandoned native snapshot")?;
                continue;
            }
            let reservation=path.join("reservation");
            match std::fs::symlink_metadata(&reservation) {
                Ok(metadata) if metadata.is_file() && metadata.len()==8=> {
                    use std::io::Read;let mut bytes=[0;8];
                    std::fs::File::open(&reservation).and_then(|mut file|file.read_exact(&mut bytes))
                        .map_err(|_|"storage_pressure: native snapshot reservation is incomplete")?;
                    reserved=u64::from_le_bytes(bytes);
                },
                Ok(_)=>return Err("storage_pressure: native snapshot reservation is invalid".into()),
                Err(error) if error.kind()==std::io::ErrorKind::NotFound=> {
                    let size=owner.metadata().map_err(|_|"storage_pressure: native snapshot reservation is unavailable")?.len();
                    if size==8 {use std::io::Read;let mut bytes=[0;8];(&owner).read_exact(&mut bytes).map_err(|_|"storage_pressure: native snapshot reservation is incomplete")?;reserved=u64::from_le_bytes(bytes);}
                    else if size!=0 {return Err("storage_pressure: native snapshot reservation is invalid".into());}
                },
                Err(_)=>return Err("storage_pressure: native snapshot reservation is unavailable".into()),
            }
        }
        let mut bytes=0u64;
        for name in ["history.sqlite3","history.sqlite3-wal","history.sqlite3-shm"] {
            match std::fs::symlink_metadata(path.join(name)) {
                Ok(metadata) if metadata.is_file()=>bytes=bytes.saturating_add(metadata.len()),
                Err(error) if error.kind()==std::io::ErrorKind::NotFound=>{},
                _=>return Err("storage_pressure: native snapshot data cannot be accounted for".into()),
            }
        }
        files.insert(path,bytes.max(reserved));
    }Ok(files)
}

pub(super) fn refresh_snapshots(store:&Store) -> Result<(),String> {
    let parent=store.path.parent().unwrap().join("native-snapshots");
    let tasks=store.path.parent().unwrap().join("tasks");
    let current=scratch_files(store.path.parent().unwrap().parent().unwrap())?;let mut scratch=store.scratch.lock().unwrap();
    scratch.retain(|path,_|!path.starts_with(&parent) && !(path.starts_with(&tasks) && path.file_name().and_then(|name|name.to_str()).is_some_and(|name|name.starts_with("owned-"))) || current.contains_key(path));
    for (path,bytes) in current {let reserved=scratch.entry(path).or_default();*reserved=(*reserved).max(bytes);}Ok(())
}

#[derive(Debug)]
pub(super) struct Pin {key:String,pins:Arc<Mutex<HashMap<String,usize>>>}
impl Pin {
    pub(super) fn new(store:&Store,key:String) -> Arc<Self> {
        *store.pins.lock().unwrap().entry(key.clone()).or_default()+=1;
        Arc::new(Self {key,pins:store.pins.clone()})
    }
}
impl Drop for Pin {
    fn drop(&mut self) {
        let mut pins=self.pins.lock().unwrap();
        if let Some(count)=pins.get_mut(&self.key) {*count-=1;if *count==0 {pins.remove(&self.key);}}
    }
}

fn bytes(conn:&Connection) -> Result<u64,String> {
    let pages:u64=conn.query_row("PRAGMA page_count",[],|row|row.get(0)).map_err(sql)?;
    let size:u64=conn.query_row("PRAGMA page_size",[],|row|row.get(0)).map_err(sql)?;
    Ok(pages.saturating_mul(size))
}

pub(super) fn check_wal(store:&Store) -> Result<(),String> {
    let path=std::path::PathBuf::from(format!("{}-wal",store.path.to_string_lossy()));
    let large=||std::fs::metadata(&path).is_ok_and(|metadata|metadata.len()>WAL_RESERVE);
    if !large() {return Ok(());}
    let Ok(conn)=store.connect() else {return Ok(());};
    // The file keeps its largest size, and a concurrent writer makes a truncating checkpoint fail
    // without retaining anything. Pressure is only the frames that cannot be copied back yet: those
    // written after the oldest open reader's snapshot.
    let _=conn.busy_timeout(Duration::ZERO);
    let checkpoint=|mode:&str| conn.query_row(&format!("PRAGMA wal_checkpoint({mode})"),[],|row|Ok((row.get::<_,i64>(1)?,row.get::<_,i64>(2)?))).ok();
    let Some((frames,copied))=checkpoint("PASSIVE") else {return Ok(());};
    let page:u64=conn.query_row("PRAGMA page_size",[],|row|row.get(0)).map_err(sql)?;
    // A checkpoint that could not run reports -1; another connection's checkpoint is then in progress.
    let retained=if frames<0 || copied<0 {0} else {(frames-copied).max(0) as u64};
    if retained.saturating_mul(page+24)>WAL_RESERVE {
        return Err("storage_pressure: active history readers are retaining too much derived data; retry after those reads finish".into());
    }
    // Everything was copied back: shrink the file when no other connection is using it.
    if frames>=0 && frames==copied {let _=checkpoint("TRUNCATE");}
    Ok(())
}

pub(super) fn trim(store:&Store,conn:&Connection) -> Result<(),String> {
    let _gate=store.cache_gate.lock().unwrap();trim_locked(store,conn)
}
pub(super) fn trim_locked(store:&Store,conn:&Connection) -> Result<(),String> {
    refresh_snapshots(store)?;
    trim_to(store,conn,store.budget().saturating_sub((WAL_RESERVE+16*1024*1024).saturating_add(store.scratch_bytes())))
}

fn trim_to(store:&Store,conn:&Connection,limit:u64) -> Result<(),String> {
    if bytes(conn)?<=limit {return Ok(());}
    // Pages freed by replaced or deleted rows count against the budget until they are returned;
    // reclaim them before evicting any indexed generation.
    super::store::reclaim_free_pages(conn).map_err(sql)?;
    if bytes(conn)?<=limit {return Ok(());}
    let mut protected=store.pins.lock().unwrap().keys().cloned().collect::<HashSet<_>>();
    {
        let mut readers=store.readers.lock().unwrap();readers.retain(|_,reader|reader.touched.elapsed()<Duration::from_secs(600));
        for reader in readers.values() {protected.extend(reader.sources.iter().cloned());}
    }
    protected.extend(store.jobs.lock().unwrap().keys().cloned());
    // Select only bounded identities. Projections, checkpoints and original bodies stay on disk.
    let mut after:Option<(i64,String,String)>=None;
    loop {
    let candidates={
        let mut statement=conn.prepare("SELECT touched,source_id,generation FROM generation_access WHERE ?1 IS NULL OR (touched,source_id,generation)>(?1,?2,?3)
            ORDER BY touched,source_id,generation LIMIT 128").map_err(sql)?;
        let result=statement.query_map(params![after.as_ref().map(|item|item.0),after.as_ref().map(|item|&item.1),after.as_ref().map(|item|&item.2)],
            |row|Ok((row.get::<_,i64>(0)?,row.get::<_,String>(1)?,row.get::<_,String>(2)?))).map_err(sql)?.collect::<Result<Vec<_>,_>>().map_err(sql)?;result
    };
    if candidates.is_empty() {break;}
    for (touched,source,generation) in candidates {
        after=Some((touched,source.clone(),generation.clone()));
        if protected.contains(&format!("{source}:{generation}")) {continue;}
        conn.execute_batch("BEGIN IMMEDIATE").map_err(sql)?;
        let result=(|| {
            conn.execute("DELETE FROM append_fields WHERE ref IN (SELECT ref FROM fields WHERE source_id=?1 AND generation=?2)",params![source,generation]).map_err(sql)?;
            for table in ["records","fields","checkpoints","source_blocks","rows","row_parts","row_fields","files","native_pending","generation_access","protocol_aliases","protocol_state","native_db_entries","superseded_rows","branch_marks"] {
                conn.execute(&format!("DELETE FROM {table} WHERE source_id=?1 AND generation=?2"),params![source,generation]).map_err(sql)?;
            }
            // A current generation that was evicted starts again at a durable record boundary. Old
            // generations still held by a request were excluded above and remain readable until release.
            conn.execute("UPDATE sources SET generation=?3,indexed_bytes=0,status='partial',error=NULL WHERE id=?1 AND generation=?2",
                params![source,generation,uuid::Uuid::new_v4().to_string()]).map_err(sql)?;Ok(())
        })();
        if result.is_err() {let _=conn.execute_batch("ROLLBACK");return result;}
        conn.execute_batch("COMMIT").map_err(sql)?;super::store::reclaim_free_pages(conn).map_err(sql)?;
        if bytes(conn)?<=limit {return Ok(());}
    }
    }
    Err("storage_pressure: derived history cache is full and its remaining generations are in use".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn restart_accounts_for_owned_native_snapshots_and_removes_abandoned_copies() {
        let temp=super::super::tests::Temp::new();let parent=temp.0.join("chat-index/native-snapshots");std::fs::create_dir_all(&parent).unwrap();
        let active=crate::agent::kiro_store::snapshot::TempDir::in_parent(Some(&parent)).unwrap();
        std::fs::write(active.0.join("history.sqlite3"),b"Native copy").unwrap();
        let abandoned=parent.join(format!("velaterm-kiro-{}",uuid::Uuid::new_v4()));std::fs::create_dir(&abandoned).unwrap();
        std::fs::write(abandoned.join("owner.lock"),b"").unwrap();std::fs::write(abandoned.join("history.sqlite3"),b"Abandoned copy").unwrap();
        let foreign=parent.join("unowned");std::fs::create_dir(&foreign).unwrap();std::fs::write(foreign.join("original"),b"Keep").unwrap();
        let files=scratch_files(&temp.0).unwrap();assert_eq!(files.get(&active.0),Some(&11));assert!(active.0.exists());assert!(!abandoned.exists());assert!(foreign.exists());
        active.reserve(WAL_RESERVE).unwrap();let files=scratch_files(&temp.0).unwrap();assert_eq!(files.get(&active.0),Some(&WAL_RESERVE));
        active.reserve(WAL_RESERVE+1024).unwrap();let files=scratch_files(&temp.0).unwrap();assert_eq!(files.get(&active.0),Some(&(WAL_RESERVE+1024)));
        let path=active.0.clone();drop(active);assert!(!path.exists());
    }
    #[test]
    fn lru_continues_past_a_full_batch_of_protected_generations() {
        let temp=super::super::tests::Temp::new();let branch=super::super::tests::indexed(&temp,"{\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"Original\"}]}}\n");
        let store=branch.store.clone();let source=branch.sources[0].id.clone();let conn=store.connect().unwrap();let mut pins=Vec::new();
        for index in 0..128 {
            let source=format!("protected-{index:03}");conn.execute("INSERT INTO generation_access VALUES(?1,'generation',0)",[&source]).unwrap();
            pins.push(Pin::new(&store,format!("{source}:generation")));
        }
        drop(branch);let _=trim_to(&store,&conn,0);
        assert_eq!(conn.query_row("SELECT count(*) FROM rows WHERE source_id=?1",[source],|row|row.get::<_,u64>(0)).unwrap(),0);
        assert_eq!(conn.query_row("SELECT count(*) FROM generation_access WHERE generation='generation'",[],|row|row.get::<_,u64>(0)).unwrap(),128);drop(pins);
    }

    #[test]
    fn summary_scratch_reserves_the_shared_budget_before_file_growth() {
        let temp=super::super::tests::Temp::new();let branch=super::super::tests::indexed(&temp,"{\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"Original\"}]}}\n");
        let store=branch.store.clone();let path=temp.0.join("chat-index/tasks/test.summary");let native=branch.sources[0].segment.path.clone();
        assert!(store.reserve_scratch(&path,store.budget()).unwrap_err().starts_with("storage_pressure:"));assert_eq!(store.scratch_bytes(),0);
        store.reserve_scratch(&path,1024).unwrap();assert_eq!(store.scratch_bytes(),1024);store.release_scratch(&path);assert_eq!(store.scratch_bytes(),0);
        assert!(std::fs::read_to_string(native).unwrap().contains("Original"));
    }

    #[test]
    fn eviction_preserves_request_pins_subscriptions_and_unique_originals() {
        let temp=super::super::tests::Temp::new();let branch=super::super::tests::indexed(&temp,"{\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"Original\"}]}}\n");
        let store=branch.store.clone();let source=branch.sources[0].id.clone();let path=branch.sources[0].segment.path.clone();
        let original=std::fs::read(&path).unwrap();let spool=temp.0.join("chat-spool");std::fs::create_dir_all(&spool).unwrap();std::fs::write(spool.join("unique.jsonl"),b"Only copy").unwrap();
        let reader=store.subscribe("owner",&branch.sources,None).unwrap();let conn=store.connect().unwrap();
        assert!(trim_to(&store,&conn,0).unwrap_err().starts_with("storage_pressure:"));
        drop(branch);assert!(trim_to(&store,&conn,0).is_err());
        assert_eq!(conn.query_row("SELECT count(*) FROM rows WHERE source_id=?1",[&source],|row|row.get::<_,u64>(0)).unwrap(),1);
        store.cancel_reader("owner",&reader).unwrap();let _=trim_to(&store,&conn,0);
        assert_eq!(conn.query_row("SELECT count(*) FROM rows WHERE source_id=?1",[&source],|row|row.get::<_,u64>(0)).unwrap(),0);
        assert_eq!(conn.query_row("SELECT indexed_bytes FROM sources WHERE id=?1",[source],|row|row.get::<_,u64>(0)).unwrap(),0);
        assert_eq!(std::fs::read(path).unwrap(),original);assert_eq!(std::fs::read(spool.join("unique.jsonl")).unwrap(),b"Only copy");
    }
}

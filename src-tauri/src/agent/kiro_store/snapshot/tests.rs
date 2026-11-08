use super::*;

#[test]
fn wal_snapshot_reads_committed_frames_without_changing_native_files() {
    let f=crate::agent::kiro_store::tests::Fixture::new();
    let db=f.db();db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0;").unwrap();
    f.insert(&db,"one",&f.dir,"Uncheckpointed text");
    let path=f.roots.database.as_ref().unwrap();
    let before=[path.clone(),sidecar(path,"-wal"),sidecar(path,"-shm")].map(|p| fs::read(p).unwrap());
    let temporary;
    {
        let copy=database(path).unwrap();temporary=copy._directory.0.clone();
        let text:String=copy.conn.query_row("SELECT value FROM conversations_v2 WHERE conversation_id='one'",[],|r|r.get(0)).unwrap();
        assert!(text.contains("Uncheckpointed text"));
        assert!(temporary.exists());
    }
    assert!(!temporary.exists());
    let after=[path.clone(),sidecar(path,"-wal"),sidecar(path,"-shm")].map(|p| fs::read(p).unwrap());
    assert_eq!(before,after);
}

#[test]
fn concurrent_changes_are_bounded_and_do_not_produce_a_mixed_snapshot() {
    let f=crate::agent::kiro_store::tests::Fixture::new();let db=f.db();
    db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0;").unwrap();
    let mut attempts=0;let mut snapshots=Vec::new();
    let result=database_with_probe(f.roots.database.as_ref().unwrap(),|path| {snapshots.push(path.to_path_buf());attempts+=1;f.insert(&db,"one",&f.dir,&format!("generation {attempts}"));});
    assert!(result.err().unwrap().contains("keeps changing"));assert_eq!(attempts,3);
    assert!(snapshots.iter().all(|path| !path.exists()));
}

#[test]
fn rollback_journal_and_corrupt_database_fail_without_recovery() {
    let f=crate::agent::kiro_store::tests::Fixture::new();drop(f.db());let path=f.roots.database.as_ref().unwrap();
    fs::write(sidecar(path,"-journal"),b"pending").unwrap();let before=fs::read(path).unwrap();
    assert!(database(path).err().unwrap().contains("rollback journal"));assert_eq!(fs::read(path).unwrap(),before);
    fs::remove_file(sidecar(path,"-journal")).unwrap();fs::write(path,b"not sqlite").unwrap();
    assert!(database(path).is_err());assert_eq!(fs::read(path).unwrap(),b"not sqlite");
}

#[test]
fn streamed_generation_exceeds_the_old_memory_limit_and_detects_same_size_edits() {
    let f=crate::agent::kiro_store::tests::Fixture::new();let path=f.dir.join("large-native");
    let mut file=File::create(&path).unwrap();file.set_len(MAX_SOURCE_BYTES+1).unwrap();file.write_all(b"original").unwrap();drop(file);
    let directory=TempDir::new().unwrap();let copy=directory.0.join("copy");
    let first=generation(&path,Some(&copy),1024*1024*1024).unwrap().unwrap();
    assert_eq!(first.bytes,MAX_SOURCE_BYTES+1);assert_eq!(fs::metadata(&copy).unwrap().len(),first.bytes);
    assert!(first==generation(&path,None,1024*1024*1024).unwrap().unwrap());
    let mut file=fs::OpenOptions::new().write(true).open(&path).unwrap();file.write_all(b"modified").unwrap();drop(file);
    assert!(first!=generation(&path,None,1024*1024*1024).unwrap().unwrap());
    let over=directory.0.join("over");
    assert!(generation(&path,Some(&over),1024).err().unwrap().starts_with("storage_pressure:"));assert!(!over.exists());
    #[cfg(unix)] {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(fs::metadata(copy).unwrap().permissions().mode()&0o777,0o600);
    }
}

#[test]
fn cancellation_stops_a_copy_at_a_block_boundary_and_preserves_the_native_file() {
    let f=crate::agent::kiro_store::tests::Fixture::new();let path=f.dir.join("large-native");
    File::create(&path).unwrap().set_len(64*1024*1024).unwrap();let directory=TempDir::new().unwrap();let copy=directory.0.join("copy");
    let cancel=AtomicBool::new(false);
    let error=std::thread::scope(|scope| {
        scope.spawn(|| {
            while !cancel.load(Ordering::Relaxed) {
                if fs::metadata(&copy).is_ok_and(|metadata|metadata.len()>0) {cancel.store(true,Ordering::Relaxed);break;}
                std::thread::park_timeout(std::time::Duration::from_millis(1));
            }
        });
        let result=generation_with_cancel(&path,Some(&copy),1024*1024*1024,&cancel);cancel.store(true,Ordering::Relaxed);result.err().unwrap()
    });
    assert_eq!(error,"read_canceled");assert!(fs::metadata(copy).unwrap().len()<64*1024*1024);assert_eq!(fs::metadata(path).unwrap().len(),64*1024*1024);
}

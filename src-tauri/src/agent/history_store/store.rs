//! Persistent derived metadata, with one scan per physical source and independent bounded reads.

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration,Instant};

use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};

use crate::models::SessionKind;
use super::limits::*;
use super::projection::{Checkpoint, Projector};
use super::source::{self, Segment};

pub(super) fn sql(error: rusqlite::Error) -> String {
    match &error {
        rusqlite::Error::SqliteFailure(code,_) if code.code==rusqlite::ErrorCode::DiskFull=>"storage_pressure: derived history storage is full".into(),
        rusqlite::Error::SqliteFailure(code,_) if matches!(code.code,rusqlite::ErrorCode::DatabaseBusy|rusqlite::ErrorCode::DatabaseLocked)=>"index_busy: history storage is busy; retry this read".into(),
        _=>format!("History index failed: {error}"),
    }
}
pub(super) fn digest(value: &[u8]) -> String {
    Sha256::digest(value).iter().map(|byte| format!("{byte:02x}")).collect()
}

#[derive(Clone, Debug)]
pub struct RegisteredSource {
    pub id: String,
    pub generation: String,
    pub segment: Segment,
    pub size: u64,
    pub indexed: u64,
    pub status: String,
    pub(super) database:Option<super::native_database::Snapshot>,
    pub(super) kiro:Option<super::kiro_database::Source>,
    pub(super) kiro_pair:Option<Box<super::kiro_pair::Source>>,
    pub(super) _pin:Arc<super::cache::Pin>,
}

pub struct Store {
    pub path: PathBuf,
    pub(super) lease:Option<File>,
    pub(super) jobs: Mutex<HashMap<String, Arc<AtomicBool>>>,
    pub(super) readers:Mutex<super::readers::Readers>,
    pub(super) cursor_key: String,
    pub(super) pins:Arc<Mutex<HashMap<String,usize>>>,
    /// Derived-data budget. It starts at the configured size and grows to fit the largest single
    /// recording opened in this process, so one long conversation is never too big to read.
    pub(super) cache_budget:std::sync::atomic::AtomicU64,
    pub(super) cache_gate:Mutex<()>,
    pub(super) scratch:Mutex<HashMap<PathBuf,u64>>,
}

/// A scan commits after this much recording or this long, whichever comes first, so a reader
/// waiting on the watermark still sees steady progress.
const COMMIT_BYTES:u64=1024*1024;
const COMMIT_INTERVAL:Duration=Duration::from_millis(50);

type Stores=HashMap<PathBuf,(Arc<Store>,Instant)>;
static STORES: OnceLock<Mutex<Stores>> = OnceLock::new();

fn idle(store:&Arc<Store>) -> bool {
    if Arc::strong_count(store)!=1 || !store.jobs.lock().unwrap().is_empty() {return false;}
    let mut readers=store.readers.lock().unwrap();readers.retain(|_,reader|reader.touched.elapsed()<Duration::from_secs(600));readers.is_empty()
}
fn stores() -> &'static Mutex<Stores> {
    STORES.get_or_init(|| {
        std::thread::spawn(||loop {
            std::thread::park_timeout(Duration::from_secs(30));
            if let Some(stores)=STORES.get() {
                stores.lock().unwrap().retain(|_,(store,touched)|touched.elapsed()<Duration::from_secs(60) || !idle(store));
            }
        });Mutex::new(HashMap::new())
    })
}

struct ScanReader<'a> {
    file: &'a mut File,
    conn: &'a Connection,
    source: &'a RegisteredSource,
    cancel: &'a AtomicBool,
    offset: u64,
}

impl Read for ScanReader<'_> {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        if self.cancel.load(Ordering::Relaxed) {return Err(std::io::Error::other("read_canceled"));}
        let count=self.file.read(bytes)?;
        if count>0 {
            // A resumed read can overlap a previously committed input block. Verify that block
            // against this open handle before replacing its hash or publishing new projections.
            let mut stmt=self.conn.prepare("SELECT start,end,hash FROM source_blocks WHERE source_id=?1 AND generation=?2 AND start<?4 AND end>?3")
                .map_err(std::io::Error::other)?;
            let blocks=stmt.query_map(params![self.source.id,self.source.generation,self.offset,self.offset+count as u64],
                |r|Ok((r.get::<_,u64>(0)?,r.get::<_,u64>(1)?,r.get::<_,Vec<u8>>(2)?))).map_err(std::io::Error::other)?;
            let resume=self.file.stream_position()?;
            for block in blocks {
                let (start,end,hash)=block.map_err(std::io::Error::other)?;
                if end<start || end-start>INPUT_BYTES as u64 {return Err(std::io::Error::other("History block exceeds its byte budget"));}
                self.file.seek(SeekFrom::Start(start))?;
                let mut previous=vec![0;(end-start) as usize]; self.file.read_exact(&mut previous)?;
                if Sha256::digest(&previous).as_slice()!=hash {return Err(std::io::Error::other("source_changed: indexed history block changed"));}
            }
            self.file.seek(SeekFrom::Start(resume))?;
            self.conn.execute("INSERT OR REPLACE INTO source_blocks VALUES(?1,?2,?3,?4,?5)",
                params![self.source.id,self.source.generation,self.offset,self.offset+count as u64,Sha256::digest(&bytes[..count]).to_vec()])
                .map_err(std::io::Error::other)?;
            self.offset+=count as u64;
        }
        Ok(count)
    }
}

impl Store {
    pub fn at(data_dir: &Path) -> Result<Arc<Self>, String> {
        let path = data_dir.join("chat-index").join("index.sqlite");
        let stores = stores();
        let mut cache=stores.lock().unwrap();
        if let Some((store,touched))=cache.get_mut(&path) {*touched=Instant::now();return Ok(store.clone());}
        if cache.len()>=64 {
            let oldest=cache.iter().filter(|(_,entry)|idle(&entry.0)).min_by_key(|(_,entry)|entry.1).map(|(path,_)|path.clone());
            if let Some(oldest)=oldest {cache.remove(&oldest);} else {return Err("index_busy: too many history stores are in use".into());}
        }
        std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| format!("History storage is unavailable: {e}"))?;
        let mut store = Self { path: path.clone(),lease:None, jobs: Mutex::new(HashMap::new()),readers:Mutex::new(HashMap::new()),cursor_key:String::new(),
            pins:Arc::new(Mutex::new(HashMap::new())),cache_budget:std::sync::atomic::AtomicU64::new(super::cache::budget()),cache_gate:Mutex::new(()),scratch:Mutex::new(super::cache::scratch_files(data_dir)?) };
        let (conn,lease) = super::recovery::open(&path,data_dir)?;store.lease=Some(lease);
        let version: u32 = conn.query_row("SELECT version FROM index_meta", [], |row| row.get(0)).map_err(sql)?;
        if version != SCHEMA_VERSION { return Err("History index schema requires a supported migration".into()); }
        // Cache accounting includes generations produced before the LRU ledger was introduced.
        // These queries visit indexed identities, never materialize original bodies or projections.
        // They read every table in full, so they run once per index file rather than on every open.
        let accounted:i64=conn.query_row("PRAGMA user_version",[],|row|row.get(0)).map_err(sql)?;
        if accounted<1 {
            for table in ["records","fields","checkpoints","source_blocks"] {
                conn.execute(&format!("INSERT OR IGNORE INTO generation_access SELECT source_id,generation,?1 FROM {table} GROUP BY source_id,generation"),
                    [super::cache::now()]).map_err(sql)?;
            }
            conn.execute_batch("PRAGMA user_version=1").map_err(sql)?;
        }
        conn.execute("INSERT OR IGNORE INTO cursor_keys VALUES(1,?1)",[uuid::Uuid::new_v4().to_string()]).map_err(sql)?;
        store.cursor_key=conn.query_row("SELECT secret FROM cursor_keys WHERE id=1",[],|r|r.get(0)).map_err(sql)?;
        let store=Arc::new(store);
        cache.insert(path,(store.clone(),Instant::now()));Ok(store)
    }

    pub fn connect(&self) -> Result<Connection, String> {
        let conn = Connection::open(&self.path).map_err(sql)?;
        conn.busy_timeout(Duration::from_secs(2)).map_err(sql)?;
        conn.execute_batch("PRAGMA cache_size=-8192; PRAGMA foreign_keys=ON;").map_err(sql)?;
        // Without a limit the WAL keeps its largest size after every checkpoint, which reads as pressure.
        conn.pragma_update(None,"journal_size_limit",(super::cache::WAL_RESERVE/4) as i64).map_err(sql)?;
        let page_size:u64=conn.query_row("PRAGMA page_size",[],|row|row.get(0)).map_err(sql)?;
        conn.pragma_update(None,"max_page_count",self.budget().saturating_sub(super::cache::WAL_RESERVE.saturating_add(self.scratch_bytes()))/page_size).map_err(sql)?;
        Ok(conn)
    }

    pub(super) fn budget(&self) -> u64 {self.cache_budget.load(std::sync::atomic::Ordering::Relaxed)}

    /// Measured on real recordings the index takes about 1.6 times the recording; twice its size
    /// plus the fixed reserves leaves room for one such recording beside nothing else.
    fn fit_budget(&self,recording:u64) {
        let needed=recording.saturating_mul(2).saturating_add(super::cache::WAL_RESERVE+64*1024*1024);
        self.cache_budget.fetch_max(needed,std::sync::atomic::Ordering::Relaxed);
    }

    pub(super) fn scratch_bytes(&self) -> u64 {self.scratch.lock().unwrap().values().fold(0u64,|total,bytes|total.saturating_add(*bytes))}

    pub(crate) fn reserve_scratch(&self,path:&Path,bytes:u64) -> Result<(),String> {
        let _gate=self.cache_gate.lock().unwrap();
        super::cache::refresh_snapshots(self)?;
        let previous=self.scratch.lock().unwrap().insert(path.to_owned(),bytes);
        let result=self.connect().and_then(|conn|super::cache::trim_locked(self,&conn));
        if result.is_err() {
            let mut scratch=self.scratch.lock().unwrap();
            if let Some(previous)=previous {scratch.insert(path.to_owned(),previous);} else {scratch.remove(path);}
        }
        result
    }
    pub(crate) fn release_scratch(&self,path:&Path) {self.scratch.lock().unwrap().remove(path);}

    pub fn register(&self, kind: SessionKind, segment: Segment) -> Result<RegisteredSource, String> {
        let mut file = File::open(&segment.path).map_err(|e| format!("History source is unavailable: {e}"))?;
        let stamp = source::stamp(&mut file)?;
        self.fit_budget(stamp.size);
        let _cache=self.cache_gate.lock().unwrap();
        let conn = self.connect()?;
        let previous: Option<(String, String, String, u64, String, Vec<u8>, u64, String, String, String, u32)> = conn.query_row(
            "SELECT id,generation,identity,size,modified,head_hash,indexed_bytes,status,provider,native_id,parser_version FROM sources WHERE path=?1",
            [segment.path.to_string_lossy().as_ref()], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?,r.get(9)?,r.get(10)?)),
        ).optional().map_err(sql)?;
        let (id, generation, indexed, status) = if let Some((id, generation, identity, size, modified, hash, indexed, status, provider, native, version)) = previous {
            file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
            let mut old_head = vec![0; size.min(64 * 1024) as usize];
            let prefix_same = file.read_exact(&mut old_head).is_ok() && Sha256::digest(&old_head).as_slice() == hash;
            let same = provider==kind.as_str() && native==segment.native_id && version==PARSER_VERSION
                && identity == stamp.identity && stamp.size >= size && prefix_same
                && (stamp.size > size || modified == stamp.modified);
            if same { (id, generation, indexed, if stamp.size > size { "partial".into() } else { status }) }
            else { (id, uuid::Uuid::new_v4().to_string(), 0, "partial".into()) }
        } else { (uuid::Uuid::new_v4().to_string(), uuid::Uuid::new_v4().to_string(), 0, "partial".into()) };
        conn.execute("INSERT INTO sources(id,provider,native_id,path,identity,generation,size,modified,head_hash,indexed_bytes,status,parser_version)
            VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)
            ON CONFLICT(path) DO UPDATE SET provider=excluded.provider,native_id=excluded.native_id,parser_version=excluded.parser_version,
            identity=excluded.identity,generation=excluded.generation,size=excluded.size,
            modified=excluded.modified,head_hash=excluded.head_hash,indexed_bytes=excluded.indexed_bytes,status=excluded.status,error=NULL",
            params![id,kind.as_str(),segment.native_id,segment.path.to_string_lossy(),stamp.identity,generation,stamp.size,
                stamp.modified,stamp.head_hash,indexed,status,PARSER_VERSION]).map_err(sql)?;
        for (key,cancel) in self.jobs.lock().unwrap().iter() {
            if key.starts_with(&format!("{id}:")) && key!=&format!("{id}:{generation}") {cancel.store(true,Ordering::Relaxed);}
        }
        let _pin=super::cache::Pin::new(self,format!("{id}:{generation}"));
        conn.execute("INSERT INTO generation_access VALUES(?1,?2,?3) ON CONFLICT(source_id,generation) DO UPDATE SET touched=excluded.touched",
            params![id,generation,super::cache::now()]).map_err(sql)?;
        super::cache::trim_locked(self,&conn)?;
        Ok(RegisteredSource { id, generation, segment, size: stamp.size, indexed, status, database:None,kiro:None,kiro_pair:None, _pin })
    }

    pub fn schedule(self: &Arc<Self>, kind: SessionKind, source: RegisteredSource) {
        if source.segment.end_byte.is_some_and(|end|source.indexed>=end) && (source.kiro_pair.is_none() || source.status=="complete") {return;}
        if source.status == "complete" && source.indexed >= source.size { return; }
        let key = format!("{}:{}", source.id, source.generation);
        let cancel = {
            let mut jobs = self.jobs.lock().unwrap();
            // Replacement generations wait for the old physical reader to leave. Two generations
            // must never concurrently scan the same source, even when one has been canceled.
            if jobs.keys().any(|key|key.starts_with(&format!("{}:",source.id))) { return; }
            let cancel = Arc::new(AtomicBool::new(false));
            jobs.insert(key.clone(), cancel.clone());
            cancel
        };
        if !super::scheduler::schedule(self.clone(),kind,source,key.clone(),cancel) {self.scan_finished(&key);}
    }

    pub(super) fn scan_finished(&self,key:&str) {self.jobs.lock().unwrap().remove(key);}

    /// A caller can cancel its own scan handle; shared UI subscriptions do not cancel another reader.
    pub fn scan(&self, kind: SessionKind, source: &RegisteredSource, cancel: &AtomicBool, tail: bool) -> Result<(), String> {
        let _work=super::work::Work::acquire(cancel)?;
        if source.kiro_pair.is_some() {return super::kiro_pair::scan(self,source,cancel);}
        if source.kiro.is_some() {return super::kiro_database::scan(self,source,cancel);}
        if source.database.is_some() {return super::native_database::scan(self,source,cancel);}
        let conn = self.connect()?;
        // Everything a scan writes is rebuilt from the recording, so its commits need not wait for
        // the disk. WAL mode keeps the index consistent either way.
        conn.execute_batch("PRAGMA synchronous=NORMAL").map_err(sql)?;
        let mut file = File::open(&source.segment.path).map_err(|e| format!("History source is unavailable: {e}"))?;
        let protocol:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM protocol_sources WHERE source_id=?1)",[&source.id],|row|row.get(0)).map_err(sql)?;
        let stamp = source::stamp(&mut file)?;
        let expected:Option<String>=conn.query_row("SELECT identity FROM sources WHERE id=?1 AND generation=?2",
            params![source.id,source.generation],|r|r.get(0)).optional().map_err(sql)?;
        if expected.as_deref()!=Some(stamp.identity.as_str()) {return Err("source_changed: history source was replaced before scanning".into());}
        if stamp.size < source.size { return Err("source_changed: history source was truncated".into()); }
        let limit=source.segment.end_byte.unwrap_or(source.size).min(source.size);
        let mut start = if tail { limit.saturating_sub(TAIL_BYTES) } else { source.indexed.min(limit) };
        file.seek(SeekFrom::Start(start)).map_err(|e| e.to_string())?;
        if tail && start > 0 {
            // Discard only the first partial physical line, without retaining its bytes.
            let mut bytes = vec![0;INPUT_BYTES];
            while start < limit {
                let requested=(limit-start).min(INPUT_BYTES as u64) as usize;
                let count=file.read(&mut bytes[..requested]).map_err(|e| e.to_string())?;
                if count==0 {break;}
                if let Some(index)=bytes[..count].iter().position(|byte|*byte==b'\n') {
                    start+=index as u64+1; file.seek(SeekFrom::Start(start)).map_err(|e|e.to_string())?; break;
                }
                start+=count as u64;
            }
        }
        let validation_file = file.try_clone().map_err(|e|e.to_string())?;
        let reader=ScanReader {file:&mut file,conn:&conn,source,cancel,offset:start};
        let mut parser = Projector::new(reader.take(limit - start), start).long_checkpoints_only();
        let mut checkpoint_count = 0;
        let mut checkpoint_error = None;
        let mut gaps = false;
        // Records are committed in batches; a commit per record rewrote the same pages each time.
        let (mut pending_bytes,mut pending_since)=(0u64,Instant::now());
        conn.execute_batch("BEGIN IMMEDIATE").map_err(sql)?;
        let scan_result: Result<(),String> = (|| {
            loop {
                if cancel.load(Ordering::Relaxed) { return Err("read_canceled".into()); }
                let record = parser.next(&mut |checkpoint: Checkpoint| {
                    if cancel.load(Ordering::Relaxed) { return Err("read_canceled".into()); }
                    if !super::adapter::addressed(&checkpoint.path) {return Ok(());}
                    if checkpoint.sequence==0 {
                        conn.execute("DELETE FROM checkpoints WHERE source_id=?1 AND generation=?2 AND record_start=?3 AND path=?4",
                            params![source.id,source.generation,checkpoint.record_start,checkpoint.path]).map_err(sql)?;
                    }
                    let result = conn.execute("INSERT OR REPLACE INTO checkpoints VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
                        params![source.id,source.generation,checkpoint.record_start,checkpoint.path,checkpoint.raw_start,
                            checkpoint.raw_end,checkpoint.decoded_start,checkpoint.decoded_end,checkpoint.hash,checkpoint.sequence]).map_err(sql);
                    if let Err(error) = &result { checkpoint_error = Some(error.clone()); }
                    result?;
                    checkpoint_count += 1;
                    if checkpoint_count >= 128 {
                        conn.execute_batch("COMMIT").map_err(sql)?;
                        super::cache::trim(self,&conn)?;
                        super::cache::check_wal(self)?;
                        conn.execute_batch("BEGIN IMMEDIATE").map_err(sql)?;
                        checkpoint_count = 0;
                        std::thread::yield_now();
                    }
                    Ok(())
                })?;
                if let Some(error) = checkpoint_error.take() { return Err(error); }
                let Some(mut record) = record else { break; };
                if !record.complete { break; }
                if source.segment.end_ordinal.is_some_and(|end|record.value["ordinal"].as_u64().is_some_and(|ordinal|ordinal>=end)) {break;}
                let generation: String = conn.query_row("SELECT generation FROM sources WHERE id=?1", [&source.id], |r| r.get(0)).map_err(sql)?;
                if generation != source.generation { return Err("source_changed: history generation changed".into()); }
                source::opened_source_unchanged(&validation_file, &source.segment.path, &stamp)?;
                gaps |= record.error.is_some();
                let mut refs = super::adapter::store_fields(&conn, source, &record)?;
                if protocol {super::protocol_source::store_record(&conn,kind,source,&record,&refs)?;}
                else {
                    if record.error.is_none() {super::adapter::expand_arguments(&conn,source,&mut record,&mut refs)?;}
                    super::adapter::store_record(&conn, kind, source, &record, &refs)?;
                }
                if !tail {
                    conn.execute("UPDATE sources SET indexed_bytes=?3 WHERE id=?1 AND generation=?2",
                        params![source.id,source.generation,record.end]).map_err(sql)?;
                }
                pending_bytes+=record.end.saturating_sub(record.start);
                if pending_bytes<COMMIT_BYTES && pending_since.elapsed()<COMMIT_INTERVAL {continue;}
                conn.execute_batch("COMMIT").map_err(sql)?;
                (pending_bytes,pending_since)=(0,Instant::now());
                // A batch can add megabytes; trim before the hard page limit is reached.
                super::cache::trim(self,&conn)?;
                super::cache::check_wal(self)?;
                conn.execute_batch("BEGIN IMMEDIATE").map_err(sql)?;
                checkpoint_count = 0;
            }
            Ok(())
        })();
        if let Err(error) = scan_result {
            let _ = conn.execute_batch("ROLLBACK");
            return Err(error);
        }
        conn.execute_batch("COMMIT").map_err(sql)?;
        drop(parser);
        source::opened_source_unchanged(&file, &source.segment.path, &stamp)?;
        if !tail {
            let indexed: u64 = conn.query_row("SELECT indexed_bytes FROM sources WHERE id=?1", [&source.id], |r| r.get(0)).map_err(sql)?;
            let errors: u64 = conn.query_row("SELECT count(*) FROM records WHERE source_id=?1 AND generation=?2 AND error IS NOT NULL",
                params![source.id,source.generation], |r| r.get(0)).map_err(sql)?;
            let status = if gaps || errors > 0 { "error" } else if indexed >= source.size { "complete" } else { "partial" };
            conn.execute("UPDATE sources SET status=?3 WHERE id=?1 AND generation=?2", params![source.id,source.generation,status]).map_err(sql)?;
            if !protocol {
                super::adapter::select_branch(&conn, kind, source)?;
                if status=="complete" {
                    conn.execute("INSERT OR REPLACE INTO branch_marks VALUES(?1,?2,?3)",params![source.id,source.generation,indexed]).map_err(sql)?;
                }
            }
            conn.execute_batch("PRAGMA wal_checkpoint(PASSIVE)").map_err(sql)?;
        }
        Ok(())
    }
}

pub fn cancel_reader(scope:&str,id:&str) -> Result<(),String> {
    let stores=stores().lock().unwrap().values().cloned().collect::<Vec<_>>();
    for (store,_) in stores {if store.cancel_reader(scope,id)? {return Ok(());}}
    // Cancellation is idempotent after expiry or completion, including transport retries.
    Ok(())
}

#[cfg(test)]
pub(crate) fn release_test_directory(directory:&Path) {
    let path=directory.join("chat-index/index.sqlite");
    let store=stores().lock().unwrap().get(&path).map(|(store,_)|store.clone());
    if let Some(store)=store {
        for cancel in store.jobs.lock().unwrap().values() {cancel.store(true,Ordering::Relaxed);}
        let deadline=Instant::now()+Duration::from_secs(10);
        while !store.jobs.lock().unwrap().is_empty() {
            assert!(Instant::now()<deadline,"The test's owned index workers did not release their directory");
            std::thread::park_timeout(Duration::from_millis(10));
        }
    }
    super::kiro_database::release_test_directory(&path);
    stores().lock().unwrap().remove(&path);
}

/// Return every free page to the file system. `incremental_vacuum` frees one page per step, so a
/// single `execute_batch` would release just one page and leave the file growing.
pub(crate) fn reclaim_free_pages(conn:&Connection) -> rusqlite::Result<()> {
    let mut statement=conn.prepare("PRAGMA incremental_vacuum")?;let mut rows=statement.query([])?;
    while rows.next()?.is_some() {}
    Ok(())
}

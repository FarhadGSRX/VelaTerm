//! Read native files without letting SQLite create or change a native sidecar.

use std::fs::{self, File};
use std::io::{Read,Write};
use std::path::{Path, PathBuf};
use rusqlite::{Connection, OpenFlags};
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicBool,Ordering};

const MAX_SOURCE_BYTES: u64 = 128 * 1024 * 1024;

/// A private directory is dropped after the copied SQLite connection closes.
#[derive(Debug)]
pub(crate) struct TempDir(pub PathBuf,Option<File>);
impl TempDir {
    pub(super) fn new() -> Result<Self, String> {
        Self::in_parent(None)
    }
    pub(crate) fn in_parent(parent:Option<&Path>) -> Result<Self,String> {
        let path = parent.unwrap_or(&std::env::temp_dir()).join(format!("velaterm-kiro-{}", uuid::Uuid::new_v4()));
        let mut builder = fs::DirBuilder::new();
        #[cfg(unix)] {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(&path).map_err(|_| "Cannot create a private Kiro history snapshot".to_string())?;
        let lease=if parent.is_some() {
            let mut options=fs::OpenOptions::new();options.create_new(true).read(true).write(true);
            #[cfg(unix)] {use std::os::unix::fs::OpenOptionsExt;options.mode(0o600);}
            let result=options.open(path.join("owner.lock")).and_then(|file| {file.try_lock()?;Ok(file)});
            match result {Ok(file)=>Some(file),Err(_)=>{let _=fs::remove_dir_all(&path);return Err("index_busy: Kiro snapshot ownership is unavailable".into());}}
        } else {None};
        Ok(Self(path,lease))
    }
    pub(crate) fn reserve(&self,bytes:u64) -> Result<(),String> {
        self.1.as_ref().ok_or("storage_pressure: managed snapshot ownership is unavailable")?;
        // Windows byte-range locks also block reads through another handle. Keep readable metadata
        // separate from the lifetime lease and publish only a complete reservation.
        let pending=self.0.join(format!("reservation-{}.tmp",uuid::Uuid::new_v4()));
        let mut options=fs::OpenOptions::new();options.create_new(true).write(true);
        #[cfg(unix)] {use std::os::unix::fs::OpenOptionsExt;options.mode(0o600);}
        let result=(|| {
            let mut file=options.open(&pending)?;file.write_all(&bytes.to_le_bytes())?;file.sync_all()?;drop(file);
            fs::rename(&pending,self.0.join("reservation"))
        })();
        if result.is_err() {let _=fs::remove_file(&pending);}
        result.map_err(|_|"storage_pressure: native snapshot reservation cannot be persisted".into())
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {self.1.take();let _ = fs::remove_dir_all(&self.0);}
}

pub(crate) struct Database {
    pub conn: Connection,
    // Fields drop in declaration order: close SQLite before removing its directory.
    _directory: std::sync::Arc<TempDir>,
    pub copy: PathBuf,
}
impl Database {
    pub(crate) fn retain(&self) -> std::sync::Arc<TempDir> {self._directory.clone()}
}

/// Missing and unreadable sources are distinct; symlinks cannot substitute another source mid-read.
pub(super) fn bytes(path: &Path, limit: u64) -> Result<Option<Vec<u8>>, String> {
    let meta = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("Cannot inspect a Kiro history source".into()),
    };
    if !meta.is_file() { return Err("Kiro history source must be a regular file".into()); }
    if meta.len() > limit { return Err("Kiro history source exceeds the supported size limit".into()); }
    let file = File::open(path).map_err(|_| "Cannot read a Kiro history source".to_string())?;
    let opened = file.metadata().map_err(|_| "Cannot inspect an open Kiro history source".to_string())?;
    #[cfg(unix)] {
        use std::os::unix::fs::MetadataExt;
        if meta.dev() != opened.dev() || meta.ino() != opened.ino() {
            return Err("Kiro history source changed while opening it".into());
        }
    }
    let mut data = Vec::new();
    file.take(limit + 1).read_to_end(&mut data).map_err(|_| "Cannot read a Kiro history source".to_string())?;
    if data.len() as u64 > limit { return Err("Kiro history source exceeds the supported size limit".into()); }
    Ok(Some(data))
}

fn sidecar(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(suffix);
    PathBuf::from(name)
}

#[derive(PartialEq)]
struct FileGeneration { identity: String, bytes: u64, modified: Option<std::time::SystemTime>, hash: Vec<u8> }

#[cfg(test)]
fn generation(path: &Path, copy: Option<&Path>, limit: u64) -> Result<Option<FileGeneration>, String> {
    generation_with_cancel(path,copy,limit,&AtomicBool::new(false))
}
fn generation_with_cancel(path: &Path, copy: Option<&Path>, limit: u64, cancel:&AtomicBool) -> Result<Option<FileGeneration>, String> {
    if cancel.load(Ordering::Relaxed) {return Err("read_canceled".into());}
    let metadata=match fs::symlink_metadata(path) {
        Ok(metadata)=>metadata,
        Err(error) if error.kind()==std::io::ErrorKind::NotFound=>return Ok(None),
        Err(_)=>return Err("Cannot inspect a Kiro history source".into()),
    };
    if !metadata.is_file() {return Err("Kiro history source must be a regular file".into());}
    if metadata.len()>limit {return Err("storage_pressure: Kiro snapshot exceeds the derived storage budget".into());}
    let mut file=File::open(path).map_err(|_|"Cannot read a Kiro history source")?;
    let opened=file.metadata().map_err(|_|"Cannot inspect an open Kiro history source")?;
    let identity=crate::agent::history_store::source::identity(&metadata);
    if crate::agent::history_store::source::identity(&opened)!=identity {return Err("Kiro history source changed while opening it".into());}
    let mut output=copy.map(|path| {
        let mut options=fs::OpenOptions::new();options.write(true).create_new(true);
        #[cfg(unix)] {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        options.open(path).map_err(|_|"Cannot write a private Kiro history snapshot".to_string())
    }).transpose()?;
    let mut buffer=vec![0;crate::agent::history_store::limits::INPUT_BYTES];let mut hash=Sha256::new();let mut bytes=0u64;
    loop {
        if cancel.load(Ordering::Relaxed) {return Err("read_canceled".into());}
        let count=file.read(&mut buffer).map_err(|_|"Cannot read a Kiro history source")?;
        if count==0 {break;}
        bytes=bytes.checked_add(count as u64).ok_or("Kiro history byte count overflow")?;
        // A private snapshot is reconstructible and has a disk budget. Its reader never keeps the
        // complete database or WAL in memory, including during the second generation verification.
        if bytes>limit {return Err("storage_pressure: Kiro snapshot exceeds the derived storage budget".into());}
        hash.update(&buffer[..count]);
        if let Some(output)=&mut output {output.write_all(&buffer[..count]).map_err(|_|"storage_pressure: cannot save the Kiro snapshot")?;}
    }
    let current=fs::symlink_metadata(path).map_err(|_|"Kiro history source disappeared while reading")?;
    let final_opened=file.metadata().map_err(|_|"Cannot inspect an open Kiro history source")?;
    if !current.is_file() || crate::agent::history_store::source::identity(&current)!=identity
        || crate::agent::history_store::source::identity(&final_opened)!=identity || bytes!=metadata.len()
        || current.len()!=metadata.len() || final_opened.len()!=metadata.len()
        || current.modified().ok()!=metadata.modified().ok() || final_opened.modified().ok()!=metadata.modified().ok() {
        return Err("Kiro history source changed while reading it".into());
    }
    if let Some(output)=&mut output {output.sync_all().map_err(|_|"storage_pressure: cannot finish the Kiro snapshot")?;}
    Ok(Some(FileGeneration {identity,bytes,modified:metadata.modified().ok(),hash:hash.finalize().to_vec()}))
}

fn collect(path: &Path, copy: Option<&Path>, cancel:&AtomicBool,limit:u64) -> Result<(FileGeneration, Option<FileGeneration>), String> {
    // Even an apparently cold rollback journal is not safe to guess about. Never recover it in place.
    if fs::symlink_metadata(sidecar(path,"-journal")).map(|metadata|metadata.len()>0 || !metadata.is_file()).unwrap_or_else(|error|error.kind()!=std::io::ErrorKind::NotFound) {
        return Err("Kiro history has a rollback journal; retry after the native writer closes it".into());
    }
    let db=generation_with_cancel(path,copy,limit,cancel)?.ok_or("Kiro history database disappeared")?;
    let wal=generation_with_cancel(&sidecar(path,"-wal"),copy.map(|path|sidecar(path,"-wal")).as_deref(),limit.saturating_sub(db.bytes),cancel)?;
    Ok((db, wal))
}

/// Compare complete DB/WAL generations around the copy. Bounded retries never checkpoint the native DB.
pub(crate) fn database(path: &Path) -> Result<Database, String> {
    database_with_probe(path, |_| {})
}
pub(crate) fn database_with_cancel(path:&Path,cancel:&AtomicBool) -> Result<Database,String> {
    database_with_probe_cancel(path,|_|{},cancel)
}
pub(crate) fn database_reserved(path:&Path,directory:std::sync::Arc<TempDir>,limit:u64,cancel:&AtomicBool) -> Result<Database,String> {
    database_copy(path,|_|{},cancel,Some(directory),limit)
}

fn database_with_probe(path: &Path, after_copy: impl FnMut(&Path)) -> Result<Database, String> {
    database_with_probe_cancel(path,after_copy,&AtomicBool::new(false))
}
fn database_with_probe_cancel(path: &Path,after_copy: impl FnMut(&Path),cancel:&AtomicBool) -> Result<Database, String> {
    database_copy(path,after_copy,cancel,None,1024*1024*1024)
}
fn database_copy(path:&Path,mut after_copy:impl FnMut(&Path),cancel:&AtomicBool,reserved:Option<std::sync::Arc<TempDir>>,limit:u64) -> Result<Database,String> {
    for _ in 0..3 {
        let directory=match &reserved {Some(directory)=>directory.clone(),None=>std::sync::Arc::new(TempDir::new()?)};
        let copy = directory.0.join("history.sqlite3");
        if reserved.is_some() {for name in ["history.sqlite3","history.sqlite3-wal","history.sqlite3-shm"] {
            match fs::remove_file(directory.0.join(name)) {Ok(())=>{},Err(error) if error.kind()==std::io::ErrorKind::NotFound=>{},
                Err(_)=>return Err("storage_pressure: cannot reset the private native snapshot".into())}
        }}
        let first=collect(path,Some(&copy),cancel,limit)?;
        after_copy(&directory.0);
        if first!=collect(path,None,cancel,limit)? {continue;}
        // Only this private copy may create SHM or recover/checkpoint WAL pages.
        let conn = Connection::open_with_flags(&copy, OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX)
            .map_err(|_| "Kiro history database is damaged or unreadable".to_string())?;
        conn.execute_batch("PRAGMA cache_size=-8192; PRAGMA temp_store=FILE; PRAGMA query_only=ON;").map_err(|_| "Cannot protect the Kiro snapshot from writes".to_string())?;
        let finished=AtomicBool::new(false);let interrupt=conn.get_interrupt_handle();
        let result:Result<String,String>=std::thread::scope(|scope| {
            scope.spawn(|| {
                while !finished.load(Ordering::Relaxed) {
                    if cancel.load(Ordering::Relaxed) {interrupt.interrupt();}
                    std::thread::park_timeout(std::time::Duration::from_millis(25));
                }
            });
            let result=conn.query_row("PRAGMA quick_check", [], |r| r.get(0));finished.store(true,Ordering::Relaxed);
            if cancel.load(Ordering::Relaxed) {Err("read_canceled".into())} else {result.map_err(|_|"Kiro history database integrity check failed".into())}
        });let result=result?;
        if result != "ok" { return Err("Kiro history database integrity check failed".into()); }
        return Ok(Database {conn,copy,_directory:directory});
    }
    Err("Kiro history keeps changing; retry after the native writer becomes idle".into())
}

/// Metadata and events are one source. A half-written pair is never accepted as a completed empty session.
pub(super) fn pair(meta: &Path, log: &Path) -> Result<(Vec<u8>, Vec<u8>), String> {
    for _ in 0..3 {
        let first = (bytes(meta, MAX_SOURCE_BYTES)?, bytes(log, MAX_SOURCE_BYTES)?);
        let second = (bytes(meta, MAX_SOURCE_BYTES)?, bytes(log, MAX_SOURCE_BYTES)?);
        if first != second { continue; }
        return match first {
            (Some(meta), Some(log)) => Ok((meta, log)),
            _ => Err("Kiro history is incomplete: its metadata or event file is missing".into()),
        };
    }
    Err("Kiro history keeps changing; retry after the native writer becomes idle".into())
}

#[cfg(test)]
mod tests;

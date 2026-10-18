//! Bounded protocol frames and independently owned originals. Derived-index eviction never deletes these files.

use std::fs::{File,OpenOptions};
use std::io::{BufRead,BufReader,Read,Write,Seek,SeekFrom};
use std::path::{Path,PathBuf};
use std::sync::{Arc,Mutex,OnceLock};
use std::sync::atomic::{AtomicBool,Ordering};

use serde_json::json;
use sha2::{Digest,Sha256};

use super::limits::INPUT_BYTES;
use super::projection::{Checkpoint,Projector,Record};
use super::store::{sql,Store};

const DEFAULT_UNIQUE_BYTES:u64=8*1024*1024*1024;
// Original submissions include up to twenty existing 5 MiB image attachments. They are streamed
// directly to disk; their persistence budget is independent of a 512 KiB display page.
const OUTGOING_RECORD_BYTES:u64=256*1024*1024;
static ARCHIVES:OnceLock<Mutex<std::collections::HashMap<PathBuf,Arc<Archive>>>>=OnceLock::new();

pub struct Archive {
    root:PathBuf,
    data_dir:PathBuf,
    owned:Mutex<u64>,
    publishing:Mutex<()>,
    limit:u64,
}

/// A frame pins an immutable prefix of an owned append stream until a verified native source replaces it.
#[derive(Clone,Debug)]
pub struct Frame {
    pub reference:String,
    pub path:PathBuf,
    pub bytes:u64,
    pub start:u64,
    pub complete:bool,
    block_path:PathBuf,
    identity:String,
    tail_hash:Vec<u8>,
}

/// Read verified bytes from the same open original used for each block hash check.
pub struct VerifiedFrameReader<'a> {
    frame:&'a Frame,cancel:&'a AtomicBool,file:File,hashes:File,at:u64,end:u64,
    buffer:Vec<u8>,buffer_start:u64,
}
impl<'a> VerifiedFrameReader<'a> {
    pub fn new(frame:&'a Frame,start:u64,end:u64,cancel:&'a AtomicBool) -> Result<Self,String> {
        if !frame.complete || start<frame.start || end<start || end>frame.bytes {
            return Err("detail_unavailable: read range is outside its complete protocol frame".into());
        }
        let file=File::open(&frame.path).map_err(|_|"detail_unavailable: live original is unavailable")?;
        let hashes=File::open(&frame.block_path).map_err(|_|"detail_unavailable: live verification metadata is unavailable")?;
        let reader=Self {frame,cancel,file,hashes,at:start,end,buffer:Vec::new(),buffer_start:u64::MAX};
        reader.validate()?;Ok(reader)
    }
    fn validate(&self) -> Result<(),String> {
        if self.cancel.load(Ordering::Relaxed) {return Err("read_canceled".into());}
        let metadata=self.file.metadata().map_err(|_|"source_changed: live original is unavailable")?;
        let current=std::fs::metadata(&self.frame.path).map_err(|_|"source_changed: live original was removed")?;
        if super::source::identity(&metadata)!=self.frame.identity || super::source::identity(&current)!=self.frame.identity || metadata.len()<self.frame.bytes {
            return Err("source_changed: live original was replaced or truncated".into());
        }Ok(())
    }
    fn block(&mut self) -> Result<(),String> {
        self.validate()?;let start=self.at/INPUT_BYTES as u64*INPUT_BYTES as u64;
        if self.buffer_start==start {return Ok(());}
        let end=(start+INPUT_BYTES as u64).min(self.frame.bytes);
        let expected=if end-start<INPUT_BYTES as u64 {self.frame.tail_hash.clone()} else {
            let mut hash=vec![0;32];self.hashes.seek(SeekFrom::Start(start/INPUT_BYTES as u64*32)).map_err(|_|"detail_unavailable: live verification metadata is unavailable")?;
            self.hashes.read_exact(&mut hash).map_err(|_|"detail_unavailable: live verification metadata is incomplete")?;hash
        };
        self.buffer.resize((end-start) as usize,0);
        self.file.seek(SeekFrom::Start(start)).map_err(|_|"detail_unavailable: live original cannot be read")?;
        self.file.read_exact(&mut self.buffer).map_err(|_|"source_changed: live original was truncated")?;
        if Sha256::digest(&self.buffer).as_slice()!=expected {return Err("source_changed: live original content changed".into());}
        self.validate()?;self.buffer_start=start;Ok(())
    }
}
impl Read for VerifiedFrameReader<'_> {
    fn read(&mut self,output:&mut [u8]) -> std::io::Result<usize> {
        self.validate().map_err(std::io::Error::other)?;
        if self.at==self.end || output.is_empty() {return Ok(0);}
        self.block().map_err(std::io::Error::other)?;
        let offset=(self.at-self.buffer_start) as usize;
        let count=output.len().min(self.buffer.len()-offset).min((self.end-self.at) as usize);
        output[..count].copy_from_slice(&self.buffer[offset..offset+count]);self.at+=count as u64;Ok(count)
    }
}

pub fn project_frame(frame:&Frame,cancel:&AtomicBool,sink:&mut impl FnMut(Checkpoint)->Result<(),String>) -> Result<Record,String> {
    let mut parser=Projector::new(VerifiedFrameReader::new(frame,frame.start,frame.bytes,cancel)?,frame.start);
    let record=parser.next(sink)?.ok_or("index_incomplete: persisted proposal has no record")?;
    if !record.complete || record.error.is_some() || record.end!=frame.bytes {
        return Err(record.error.unwrap_or_else(||"index_incomplete: persisted proposal is incomplete".into()));
    }Ok(record)
}

pub fn read_checkpoint(frame:&Frame,checkpoint:&Checkpoint,cancel:&AtomicBool) -> Result<String,String> {
    if checkpoint.raw_end<checkpoint.raw_start || checkpoint.raw_end-checkpoint.raw_start>super::limits::CHECKPOINT_BYTES as u64+16 {
        return Err("unsupported_record: persisted proposal checkpoint exceeds its byte budget".into());
    }
    let mut reader=VerifiedFrameReader::new(frame,checkpoint.raw_start,checkpoint.raw_end,cancel)?;
    let mut bytes=vec![0;(checkpoint.raw_end-checkpoint.raw_start) as usize];
    reader.read_exact(&mut bytes).map_err(|error|error.to_string())?;reader.validate()?;
    super::projection::decode_checkpoint(&bytes)
}

impl Archive {
    pub fn at(data_dir:&Path) -> Result<Arc<Self>,String> {
        let root=data_dir.join("chat-spool");
        let archives=ARCHIVES.get_or_init(||Mutex::new(std::collections::HashMap::new()));
        let mut cache=archives.lock().unwrap();
        if let Some(archive)=cache.get(&root) {return Ok(archive.clone());}
        if cache.len()>=64 {
            let idle=cache.iter().find(|(_,archive)|Arc::strong_count(archive)==1).map(|(path,_)|path.clone());
            if let Some(idle)=idle {cache.remove(&idle);} else {return Err("storage_pressure: too many live detail stores are in use".into());}
        }
        std::fs::create_dir_all(&root).map_err(|_|"storage_pressure: live detail storage is unavailable")?;
        #[cfg(unix)] {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&root,std::fs::Permissions::from_mode(0o700)).map_err(|_|"storage_pressure: cannot protect live detail storage")?;
        }
        let mut owned=0u64;
        // The directory is the durable source of truth after a crash between an append and its ledger update.
        // Inspect names and lengths only; never read original protocol bodies during startup.
        for entry in std::fs::read_dir(&root).map_err(|_|"storage_pressure: cannot inspect live detail storage")? {
            let entry=entry.map_err(|_|"storage_pressure: cannot inspect a live detail entry")?;
            if entry.path().extension().is_some_and(|extension|matches!(extension.to_str(),Some("jsonl"|"blocks"|"json"))) {
                let metadata=entry.metadata().map_err(|_|"storage_pressure: cannot inspect a live detail entry")?;
                if metadata.is_file() {owned=owned.checked_add(metadata.len()).ok_or("storage_pressure: live detail accounting overflow")?;}
            }
        }
        let limit=std::env::var("VLX_HISTORY_UNIQUE_BYTES").ok().and_then(|value|value.parse::<u64>().ok()).filter(|value|*value>0).unwrap_or(DEFAULT_UNIQUE_BYTES);
        let archive=Arc::new(Self {root:root.clone(),data_dir:data_dir.to_owned(),owned:Mutex::new(owned),publishing:Mutex::new(()),limit});
        cache.insert(root,archive.clone()); Ok(archive)
    }

    pub fn owned_bytes(&self) -> u64 {*self.owned.lock().unwrap()}
    fn store(&self) -> Result<Arc<Store>,String> {Store::at(&self.data_dir)}

    pub fn publish(&self,frame:&Frame,kind:crate::models::SessionKind,scope:&str,epoch:u64,native:&str) -> Result<(),String> {
        self.publish_direction(frame,kind,scope,epoch,native,"incoming",true)
    }
    pub fn publish_outgoing(&self,frame:&Frame,kind:crate::models::SessionKind,scope:&str,epoch:u64,native:&str) -> Result<(),String> {
        self.publish_direction(frame,kind,scope,epoch,native,"outgoing",true)
    }
    pub fn bind_outgoing(&self,frame:&Frame,kind:crate::models::SessionKind,scope:&str,epoch:u64,native:&str) -> Result<(),String> {
        self.publish_direction(frame,kind,scope,epoch,native,"outgoing",false)
    }
    pub fn bind(&self,frame:&Frame,kind:crate::models::SessionKind,scope:&str,epoch:u64,native:&str) -> Result<(),String> {
        self.publish_direction(frame,kind,scope,epoch,native,"incoming",false)
    }
    fn publish_direction(&self,frame:&Frame,kind:crate::models::SessionKind,scope:&str,epoch:u64,native:&str,direction:&str,index:bool) -> Result<(),String> {
        if !frame.complete {return Err("index_incomplete: live original has no complete frame".into());}
        if uuid::Uuid::parse_str(&frame.reference).is_err() || scope.is_empty() || scope.len()>200 || native.is_empty() || native.len()>200
            || frame.path!=self.root.join(format!("{}.jsonl",frame.reference)) {
            return Err("detail_unavailable: protocol source identity is invalid".into());
        }
        let ownership=read_identity(&self.root.join(format!("{}.meta.json",frame.reference)))?;
        if ownership["direction"].as_str().unwrap_or("incoming")!=direction {return Err("detail_unavailable: protocol original direction does not match its publication adapter".into());}
        if ownership["reference"]!=frame.reference || ownership["sessionId"]!=scope || ownership["processEpoch"]!=epoch || ownership["version"]!=1 {
            return Err("detail_unavailable: protocol ownership does not match its authorized scope".into());
        }
        let _publishing=self.publishing.lock().unwrap();
        let identity=self.root.join(format!("{}.provider.json",frame.reference));
        let mut expected=json!({"version":1,"reference":frame.reference,"provider":kind.as_str(),"sessionId":scope,"processEpoch":epoch,"nativeId":native});
        if direction=="outgoing" {expected["direction"]=json!(direction);}
        if identity.exists() {
            if read_identity(&identity)?!=expected {return Err("detail_unavailable: published protocol identity changed".into());}
        } else {
            let bytes=serde_json::to_vec(&expected)
                .map_err(|_|"storage_pressure: cannot encode protocol identity")?;
            if bytes.len()>64*1024 {return Err("unsupported_record: protocol identity exceeds its byte budget".into());}
            let pending=self.root.join(format!("{}.provider.pending.json",frame.reference));
            let mut options=OpenOptions::new();options.create_new(true).write(true);
            #[cfg(unix)] {use std::os::unix::fs::OpenOptionsExt;options.mode(0o600);}
            // A crash can leave an unpublished identity file. Its source and ownership header are
            // independent; removing this derived identity prefix never removes original bytes.
            if pending.exists() {self.remove_owned_metadata(&pending)?;}
            let mut file=options.open(&pending).map_err(|_|"storage_pressure: cannot persist protocol identity")?;
            let persisted=self.write_owned(&mut file,&bytes).and_then(|_|file.sync_all().map_err(|_|"storage_pressure: cannot persist protocol identity".into()));drop(file);
            if let Err(error)=persisted {self.remove_owned_metadata(&pending)?;return Err(error);}
            std::fs::rename(&pending,&identity).map_err(|_|"storage_pressure: cannot publish protocol identity")?;
            #[cfg(unix)] {File::open(&self.root).and_then(|file|file.sync_all()).map_err(|_|"storage_pressure: cannot persist protocol identity directory")?;}
        }
        if !index {return Ok(());}
        let store=self.store()?;let conn=store.connect()?;
        conn.execute("INSERT INTO spool_entries VALUES(?1,?2,?3,?4,0,'unique_protocol_frame')
            ON CONFLICT(ref) DO UPDATE SET bytes=max(spool_entries.bytes,excluded.bytes)",
            rusqlite::params![frame.reference,scope,frame.path.to_string_lossy(),frame.bytes]).map_err(sql)?;
        let source=super::protocol_source::register(&store,kind,&frame.reference,scope,epoch,native)?;
        store.schedule(kind,source);Ok(())
    }

    fn remove_owned_metadata(&self,path:&Path) -> Result<(),String> {
        let bytes=std::fs::metadata(path).map_err(|_|"storage_pressure: cannot inspect unpublished protocol identity")?.len();
        std::fs::remove_file(path).map_err(|_|"storage_pressure: cannot remove unpublished protocol identity")?;
        let mut owned=self.owned.lock().unwrap();*owned=owned.saturating_sub(bytes);Ok(())
    }

    fn write_owned(&self,file:&mut File,bytes:&[u8]) -> Result<(),String> {
        let mut owned=self.owned.lock().unwrap();
        let before=file.metadata().map_err(|_|"storage_pressure: cannot inspect live detail storage")?.len();
        let position=file.stream_position().map_err(|_|"storage_pressure: cannot seek live detail storage")?;
        // A retry can overwrite a partially persisted verification hash. Only file growth consumes quota.
        let existing=before.saturating_sub(position).min(bytes.len() as u64);
        let available=self.limit.saturating_sub(*owned).saturating_add(existing).min(bytes.len() as u64) as usize;
        let result=file.write_all(&bytes[..available]);
        let after=file.metadata().map_err(|_|"storage_pressure: cannot inspect live detail storage")?.len();
        *owned=owned.checked_add(after.saturating_sub(before)).ok_or("storage_pressure: live detail accounting overflow")?;
        result.map_err(|_|"storage_pressure: live details could not be saved")?;
        if available!=bytes.len() {return Err("storage_pressure: unique live details have reached their storage limit".into());}
        Ok(())
    }

    fn begin(self:&Arc<Self>,session:&str,epoch:u64) -> Result<Recording,String> {self.begin_direction(session,epoch,"incoming")}
    fn begin_direction(self:&Arc<Self>,session:&str,epoch:u64,direction:&str) -> Result<Recording,String> {
        let reference=uuid::Uuid::new_v4().to_string();
        let path=self.root.join(format!("{reference}.jsonl"));
        let mut options=OpenOptions::new(); options.create_new(true).write(true);
        #[cfg(unix)] {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file=options.open(&path).map_err(|_|"storage_pressure: cannot create live detail storage")?;
        let block_path=self.root.join(format!("{reference}.blocks"));
        let blocks=options.open(&block_path).map_err(|_|"storage_pressure: cannot create live detail verification metadata")?;
        let identity=super::source::identity(&file.metadata().map_err(|_|"storage_pressure: cannot inspect live detail storage")?);
        let metadata=self.root.join(format!("{reference}.meta.json"));
        let mut options=OpenOptions::new(); options.create_new(true).write(true);
        #[cfg(unix)] {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut header=options.open(&metadata).map_err(|_|"storage_pressure: cannot create live detail ownership metadata")?;
        // This independent ownership header survives corruption or replacement of the reconstructible index.
        let bytes=serde_json::to_vec(&json!({"version":1,"sessionId":session,"processEpoch":epoch,"reference":reference,"direction":direction}))
            .map_err(|_|"storage_pressure: cannot encode live detail ownership metadata")?;
        if let Err(error)=self.write_owned(&mut header,&bytes) {
            drop(header);drop(blocks);drop(file);
            let mut owned=self.owned.lock().unwrap();
            for empty in [&metadata,&block_path,&path] {
                let size=std::fs::metadata(empty).map(|value|value.len()).unwrap_or(0);
                if std::fs::remove_file(empty).is_ok() {*owned=owned.saturating_sub(size);}
            }
            return Err(error);
        }
        header.sync_all().map_err(|_|"storage_pressure: cannot persist live detail ownership metadata")?;
        // The ownership header is durable independently of the reconstructible SQLite ledger.
        // Publishing and recovery rebuild that ledger off the control-receive path.
        Ok(Recording {archive:self.clone(),frame:Frame {reference,path,bytes:0,start:0,complete:false,block_path,identity,tail_hash:Vec::new()},file,blocks,
            pending:Vec::with_capacity(INPUT_BYTES),committed_blocks:0})
    }
}

pub(crate) fn read_identity(path:&Path) -> Result<serde_json::Value,String> {
    let metadata=std::fs::symlink_metadata(path).map_err(|_|"detail_unavailable: protocol identity is unavailable")?;
    if !metadata.is_file() || metadata.len()>64*1024 {return Err("detail_unavailable: protocol identity exceeds its metadata budget".into());}
    let mut bytes=Vec::with_capacity(metadata.len() as usize);
    File::open(path).map_err(|_|"detail_unavailable: protocol identity is unavailable")?.take(64*1024+1).read_to_end(&mut bytes)
        .map_err(|_|"detail_unavailable: protocol identity cannot be read")?;
    if bytes.len()>64*1024 {return Err("detail_unavailable: protocol identity exceeds its metadata budget".into());}
    serde_json::from_slice(&bytes).map_err(|_|"detail_unavailable: protocol identity is invalid".into())
}

struct Recording {archive:Arc<Archive>,frame:Frame,file:File,blocks:File,pending:Vec<u8>,committed_blocks:u64}
impl Recording {
    fn commit_blocks(&mut self) -> Result<(),String> {
        if self.pending.len()!=INPUT_BYTES {return Ok(());}
        self.blocks.seek(SeekFrom::Start(self.committed_blocks*32)).map_err(|_|"storage_pressure: cannot seek live verification metadata")?;
        self.archive.write_owned(&mut self.blocks,&Sha256::digest(&self.pending))?;
        self.committed_blocks+=1;self.pending.clear();Ok(())
    }

    fn append(&mut self,bytes:&[u8]) -> Result<(),String> {
        self.commit_blocks()?;
        // Never write more raw bytes than the pending verification block can retain. If its
        // hash cannot be persisted, all bytes already written remain available for an exact retry.
        let bytes=&bytes[..bytes.len().min(INPUT_BYTES-self.pending.len())];
        // Retain accounting for bytes actually written even when a partial write hits a full disk.
        let before=self.file.metadata().map_err(|_|"storage_pressure: cannot inspect live detail storage")?.len();
        let result=self.archive.write_owned(&mut self.file,bytes);
        let after=self.file.metadata().map_err(|_|"storage_pressure: cannot inspect live detail storage")?.len();
        let written=after.saturating_sub(before).min(bytes.len() as u64) as usize;
        self.frame.bytes=after;
        self.pending.extend_from_slice(&bytes[..written]);
        self.commit_blocks()?;
        result
    }

    fn finish(&mut self,complete:bool) -> Result<Frame,String> {
        let verification=self.commit_blocks();
        self.file.sync_data().map_err(|_|"storage_pressure: live detail persistence failed")?;
        self.blocks.sync_data().map_err(|_|"storage_pressure: live verification persistence failed")?;
        self.frame.complete=complete && verification.is_ok();
        self.frame.tail_hash=Sha256::digest(&self.pending).to_vec();
        verification?;Ok(self.frame.clone())
    }
}


struct JournalPending {key:String,hash:Vec<u8>,bytes:u64,start:u64,saved:u64}
/// User-originated records have a separate ownership header and never share a provider stdout stream.
/// Their complete bytes precede dispatch; uncertain delivery remains explicit after a lost receipt.
pub struct Journal {recording:Recording,pending:Option<JournalPending>}
impl Journal {
    pub fn pending_key(&self) -> Option<&str> {self.pending.as_ref().map(|pending|pending.key.as_str())}
    pub fn new(archive:Arc<Archive>,session:&str,epoch:u64) -> Result<Self,String> {
        if session.is_empty() || session.len()>200 {return Err("detail_unavailable: outgoing session identity is invalid".into());}
        Ok(Self {recording:archive.begin_direction(session,epoch,"outgoing")?,pending:None})
    }
    pub fn append(&mut self,key:&str,value:&impl serde::Serialize,cancel:&AtomicBool) -> Result<Frame,String> {
        if cancel.load(Ordering::Relaxed) {return Err("read_canceled".into());}
        if key.is_empty() || key.len()>256 {return Err("unsupported_record: outgoing record identity exceeds its byte budget".into());}
        let mut signature=JournalSignature {hash:Sha256::new(),bytes:0};
        serde_json::to_writer(&mut signature,value).map_err(|_|"unsupported_record: outgoing record exceeds its byte budget")?;
        signature.write_all(b"\n").map_err(|_|"unsupported_record: outgoing record exceeds its byte budget")?;
        let hash=signature.hash.finalize().to_vec();
        if let Some(pending)=&self.pending {
            if pending.key!=key || pending.hash!=hash || pending.bytes!=signature.bytes {
                return Err("storage_pressure: an earlier outgoing original must finish before another submission can be saved".into());
            }
        } else {self.pending=Some(JournalPending {key:key.into(),hash,bytes:signature.bytes,start:self.recording.frame.bytes,saved:0});}
        let pending=self.pending.as_mut().unwrap();
        self.recording.frame.start=pending.start;
        let mut writer=JournalWriter {recording:&mut self.recording,pending,skip:0,cancel};
        serde_json::to_writer(&mut writer,value).map_err(|error| {
            if cancel.load(Ordering::Relaxed) {"read_canceled".into()} else {format!("storage_pressure: outgoing original could not be saved: {error}")}
        })?;
        writer.write_all(b"\n").map_err(|_|"storage_pressure: outgoing record boundary could not be saved")?;
        if writer.pending.saved!=writer.pending.bytes {return Err("storage_pressure: outgoing original is incomplete".into());}
        let frame=self.recording.finish(true)?;self.pending=None;Ok(frame)
    }
}
struct JournalSignature {hash:Sha256,bytes:u64}
impl Write for JournalSignature {
    fn write(&mut self,bytes:&[u8]) -> std::io::Result<usize> {
        if bytes.len() as u64>OUTGOING_RECORD_BYTES.saturating_sub(self.bytes) {return Err(std::io::Error::other("Outgoing original exceeds its byte budget"));}
        self.hash.update(bytes);self.bytes+=bytes.len() as u64;Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {Ok(())}
}
struct JournalWriter<'a> {recording:&'a mut Recording,pending:&'a mut JournalPending,skip:u64,cancel:&'a AtomicBool}
impl Write for JournalWriter<'_> {
    fn write(&mut self,bytes:&[u8]) -> std::io::Result<usize> {
        if self.cancel.load(Ordering::Relaxed) {return Err(std::io::Error::other("read_canceled"));}
        let skipped=(self.pending.saved.saturating_sub(self.skip)).min(bytes.len() as u64) as usize;
        self.skip+=skipped as u64;let mut offset=skipped;
        while offset<bytes.len() {
            if self.cancel.load(Ordering::Relaxed) {return Err(std::io::Error::other("read_canceled"));}
            let before=self.recording.frame.bytes;let result=self.recording.append(&bytes[offset..]);
            let saved=self.recording.frame.bytes.saturating_sub(before);
            self.pending.saved+=saved;self.skip+=saved;offset+=saved as usize;
            result.map_err(std::io::Error::other)?;
        }Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {Ok(())}
}

/// The caller supplies a field found by the trusted projector, never a client offset or JSON pointer.
/// Verify each actual input block before writing any of its selected bytes. JSON strings retain their
/// original escapes; containers retain all keys and scalar values, including values beyond previews.
pub fn write_raw_field(frame:&Frame,field:&super::projection::Field,writer:&mut impl Write,cancel:&AtomicBool) -> Result<u64,String> {
    if cancel.load(Ordering::Relaxed) {return Err("read_canceled".into());}
    if !frame.complete || field.start<frame.start || field.end<field.start || field.end>frame.bytes || !matches!(field.encoding.as_str(),"json"|"string") {
        return Err("detail_unavailable: raw field is outside its complete protocol frame".into());
    }
    let mut file=File::open(&frame.path).map_err(|_|"detail_unavailable: live original is unavailable")?;
    let mut hashes=File::open(&frame.block_path).map_err(|_|"detail_unavailable: live verification metadata is unavailable")?;
    let verify_identity=|file:&File|->Result<(),String> {
        let metadata=file.metadata().map_err(|_|"source_changed: live original is unavailable")?;
        let current=std::fs::metadata(&frame.path).map_err(|_|"source_changed: live original was removed")?;
        if super::source::identity(&metadata)!=frame.identity || super::source::identity(&current)!=frame.identity || metadata.len()<frame.bytes {
            return Err("source_changed: live original was replaced or truncated".into());
        }
        Ok(())
    };
    verify_identity(&file)?;
    let mut at=field.start;let mut written=0;let mut opened=false;
    while at<field.end {
        if cancel.load(Ordering::Relaxed) {return Err("read_canceled".into());}
        let block=at/(INPUT_BYTES as u64);let start=block*INPUT_BYTES as u64;let end=(start+INPUT_BYTES as u64).min(frame.bytes);
        let expected=if end-start<INPUT_BYTES as u64 {frame.tail_hash.clone()} else {
            let mut hash=vec![0;32];hashes.seek(SeekFrom::Start(block*32)).map_err(|_|"detail_unavailable: live verification metadata is unavailable")?;
            hashes.read_exact(&mut hash).map_err(|_|"detail_unavailable: live verification metadata is incomplete")?;hash
        };
        file.seek(SeekFrom::Start(start)).map_err(|_|"detail_unavailable: live original cannot be read")?;
        let mut bytes=vec![0;(end-start) as usize];file.read_exact(&mut bytes).map_err(|_|"source_changed: live original was truncated")?;
        if Sha256::digest(&bytes).as_slice()!=expected {return Err("source_changed: live original content changed".into());}
        verify_identity(&file)?;
        if !opened && field.encoding=="string" {writer.write_all(b"\"").map_err(|error|error.to_string())?;written+=1;}opened=true;
        let next=end.min(field.end);writer.write_all(&bytes[(at-start) as usize..(next-start) as usize]).map_err(|error|error.to_string())?;
        written+=next-at;at=next;
    }
    if field.encoding=="string" {
        if !opened {writer.write_all(b"\"").map_err(|error|error.to_string())?;written+=1;}
        writer.write_all(b"\"").map_err(|error|error.to_string())?;written+=1;
    }
    verify_identity(&file)?;Ok(written)
}

/// The underlying buffered pipe never gives the projector bytes from the next physical frame. Its EOF
/// is a frame boundary, not process exit; the next call resumes the same pipe without an entire-line allocation.
struct FrameReader<'a,R:Read> {
    pipe:&'a mut BufReader<R>,
    recording:&'a mut Recording,
    cancel:&'a AtomicBool,
    ended:bool,
    pressure:&'a mut dyn FnMut(&str)->Result<(),String>,
}
impl<R:Read> Read for FrameReader<'_,R> {
    fn read(&mut self,output:&mut [u8]) -> std::io::Result<usize> {
        if self.ended || output.is_empty() {return Ok(0);}
        if self.cancel.load(Ordering::Relaxed) {return Err(std::io::Error::other("read_canceled"));}
        let bytes=self.pipe.fill_buf()?;
        if bytes.is_empty() {self.ended=true;return Ok(0);}
        let count=bytes.iter().position(|byte|*byte==b'\n').map_or(bytes.len(),|index|index+1).min(output.len());
        let mut saved=0;
        while saved<count {
            let before=self.recording.frame.bytes;
            match self.recording.append(&bytes[saved..count]) {
                Ok(())=>saved+=(self.recording.frame.bytes-before).min((count-saved) as u64) as usize,
                Err(error)=>{
                    // The pipe bytes remain unconsumed while storage is under pressure. The callback
                    // may wait for space; it cannot approve requests or stop the agent as a side effect.
                    saved+=(self.recording.frame.bytes-before).min((count-saved) as u64) as usize;
                    (self.pressure)(&error).map_err(std::io::Error::other)?;
                    if self.cancel.load(Ordering::Relaxed) {return Err(std::io::Error::other("read_canceled"));}
                    // Retrying consumes only the unwritten suffix. A partial disk write is not duplicated.
                    std::thread::yield_now();
                }
            }
        }
        output[..count].copy_from_slice(&bytes[..count]);
        self.ended=bytes[count-1]==b'\n'; self.pipe.consume(count); Ok(count)
    }
}

pub struct Receiver<R:Read> {pipe:BufReader<R>,archive:Arc<Archive>,session:String,epoch:u64,recording:Option<Recording>,failed:bool}
impl<R:Read> Receiver<R> {
    pub fn new(reader:R,archive:Arc<Archive>,session:&str,epoch:u64) -> Self {
        Self {pipe:BufReader::with_capacity(INPUT_BYTES,reader),archive,session:session.into(),epoch,recording:None,failed:false}
    }

    /// Start a separate original after an explicit native branch change. Buffered pipe bytes stay
    /// unread, and all references into the preceding immutable prefix remain valid.
    pub fn rotate(&mut self) -> Result<(),String> {
        if self.failed || self.recording.as_ref().is_some_and(|recording|!recording.frame.complete) {
            return Err("index_incomplete: protocol original can rotate only at a complete frame boundary".into());
        }
        self.recording=None;Ok(())
    }

    pub fn next(&mut self,cancel:&AtomicBool,sink:&mut impl FnMut(Checkpoint)->Result<(),String>,
        pressure:&mut impl FnMut(&str)->Result<(),String>) -> Result<Option<(Frame,Record)>,String> {
        if cancel.load(Ordering::Relaxed) {return Err("read_canceled".into());}
        if self.failed {return Err("detail_unavailable: protocol reception failed before a complete frame boundary".into());}
        if self.pipe.fill_buf().map_err(|e|e.to_string())?.is_empty() {return Ok(None);}
        while self.recording.is_none() {
            match self.archive.begin(&self.session,self.epoch) {Ok(recording)=>self.recording=Some(recording),Err(error)=>pressure(&error)?,}
            if cancel.load(Ordering::Relaxed) {return Err("read_canceled".into());}
        }
        let recording=self.recording.as_mut().unwrap();
        let start=recording.frame.bytes;recording.frame.start=start;
        let reader=FrameReader {pipe:&mut self.pipe,recording,cancel,ended:false,pressure};
        let result=Projector::new(reader,start).next(sink);
        match result {
            Ok(Some(record))=>{
                loop {
                    match recording.finish(record.complete) {
                        Ok(frame)=>return Ok(Some((frame,record))),
                        Err(error)=>if let Err(canceled)=pressure(&error) {self.failed=true;return Err(canceled);},
                    }
                    if cancel.load(Ordering::Relaxed) {self.failed=true;return Err("read_canceled".into());}
                }
            },
            Ok(None)=>{recording.finish(false)?;Ok(None)},
            Err(error)=>{self.failed=true;recording.finish(false)?;Err(error)},
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn archive(temp:&super::super::tests::Temp,limit:u64) -> Arc<Archive> {
        let root=temp.0.join("chat-spool");std::fs::create_dir_all(&root).unwrap();
        Arc::new(Archive {root,data_dir:temp.0.clone(),owned:Mutex::new(0),publishing:Mutex::new(()),limit})
    }

    fn disk_bytes(archive:&Archive) -> u64 {
        std::fs::read_dir(&archive.root).unwrap().map(|entry|entry.unwrap().metadata().unwrap().len()).sum()
    }

    #[test]
    fn publication_cannot_reassign_an_original_or_overwrite_a_confirmed_native_identity() {
        let temp=super::super::tests::Temp::new();let archive=archive(&temp,16*1024*1024);let raw=b"{\"method\":\"turn/completed\"}\n";
        let mut receiver=Receiver::new(raw.as_slice(),archive.clone(),"owner",9);let cancel=AtomicBool::new(false);
        let frame=receiver.next(&cancel,&mut |_|Ok(()),&mut |_|panic!("Storage is available")).unwrap().unwrap().0;
        let bytes=archive.owned_bytes();
        assert!(archive.publish(&frame,crate::models::SessionKind::Codex,"another",9,"native").unwrap_err().starts_with("detail_unavailable:"));
        assert!(archive.publish(&frame,crate::models::SessionKind::Codex,"owner",10,"native").unwrap_err().starts_with("detail_unavailable:"));
        assert!(archive.publish(&frame,crate::models::SessionKind::Codex,"owner",9,&"n".repeat(201)).unwrap_err().starts_with("detail_unavailable:"));
        assert_eq!(archive.owned_bytes(),bytes);assert!(!temp.0.join("chat-index").exists());
        archive.publish(&frame,crate::models::SessionKind::Codex,"owner",9,"native").unwrap();
        let identity=std::fs::read(archive.root.join(format!("{}.provider.json",frame.reference))).unwrap();
        assert!(archive.publish(&frame,crate::models::SessionKind::Claude,"owner",9,"native").unwrap_err().starts_with("detail_unavailable:"));
        assert!(archive.publish(&frame,crate::models::SessionKind::Codex,"owner",9,"different").unwrap_err().starts_with("detail_unavailable:"));
        assert_eq!(std::fs::read(archive.root.join(format!("{}.provider.json",frame.reference))).unwrap(),identity);
        assert_eq!(std::fs::read(frame.path).unwrap(),raw);
    }

    #[test]
    fn streamed_frames_preserve_unique_originals_without_materializing_large_bodies() {
        let temp=super::super::tests::Temp::new();let archive=archive(&temp,64*1024*1024);
        let source=super::super::tests::Giant {prefix:Cursor::new(b"{\"method\":\"item/completed\",\"params\":{\"diff\":\"".to_vec()),remaining:40*1024*1024,
            suffix:Cursor::new(b"\"}}\n{\"method\":\"turn/completed\"}\n".to_vec())};
        let mut receiver=Receiver::new(source,archive.clone(),"session",7);
        let cancel=AtomicBool::new(false);
        let (frame,record)=receiver.next(&cancel,&mut |_|Ok(()),&mut |_|panic!("Enough storage is available")).unwrap().unwrap();
        assert_eq!(record.error,None);assert!(frame.complete);assert!(frame.bytes>40*1024*1024);
        assert_eq!(record.value["params"]["diff"].as_str().unwrap().len(),super::super::limits::PREVIEW_BYTES);
        assert_eq!(archive.owned_bytes(),disk_bytes(&archive));assert!(archive.owned_bytes()>frame.bytes);
        let (control,record)=receiver.next(&cancel,&mut |_|Ok(()),&mut |_|Ok(())).unwrap().unwrap();
        assert_eq!(record.value["method"],"turn/completed");assert!(control.complete);
        assert_eq!(control.reference,frame.reference);assert_eq!(control.start,frame.bytes);assert_eq!(record.start,frame.bytes);
        assert!(receiver.next(&cancel,&mut |_|Ok(()),&mut |_|Ok(())).unwrap().is_none());
        assert!(frame.path.exists());
    }

    #[test]
    fn pressure_and_cancellation_keep_partial_unique_bytes_and_do_not_publish_a_frame() {
        let temp=super::super::tests::Temp::new();let archive=archive(&temp,super::super::limits::INPUT_BYTES as u64);
        let raw=super::super::tests::Giant {prefix:Cursor::new(b"{\"diff\":\"".to_vec()),remaining:2*super::super::limits::INPUT_BYTES,suffix:Cursor::new(b"\"}\n".to_vec())};
        let mut receiver=Receiver::new(raw,archive.clone(),"session",8);let cancel=AtomicBool::new(false);let mut pressured=false;
        let error=receiver.next(&cancel,&mut |_|Ok(()),&mut |error|{pressured=true;assert!(error.starts_with("storage_pressure:"));Err(error.to_owned())}).err().unwrap();
        assert!(pressured && error.contains("storage_pressure:"));assert!(archive.owned_bytes()>0);
        let bytes=archive.owned_bytes();cancel.store(true,Ordering::Relaxed);
        assert_eq!(receiver.next(&cancel,&mut |_|Ok(()),&mut |_|Ok(())).err().unwrap(),"read_canceled");
        assert_eq!(archive.owned_bytes(),bytes);
        let row:(u64,String)=archive.store().unwrap().connect().unwrap().query_row("SELECT bytes,reason FROM spool_entries",[],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
        assert_eq!(row.0,std::fs::metadata(&receiver.recording.as_ref().unwrap().frame.path).unwrap().len());
        assert_eq!(bytes,disk_bytes(&archive));assert!(bytes<=archive.limit);assert_eq!(row.1,"recovered_unique_original");
    }

    #[test]
    fn many_control_frames_share_one_owned_original_and_keep_absolute_record_offsets() {
        let temp=super::super::tests::Temp::new();let archive=archive(&temp,4*1024*1024);
        let raw="{\"method\":\"turn/completed\"}\n".repeat(1000);
        let mut receiver=Receiver::new(raw.as_bytes(),archive.clone(),"session",9);let cancel=AtomicBool::new(false);
        let mut end=0;let mut reference=None;let mut count=0;
        while let Some((frame,record))=receiver.next(&cancel,&mut |_|Ok(()),&mut |_|Ok(())).unwrap() {
            assert_eq!(record.start,end);assert_eq!(frame.start,record.start);assert_eq!(frame.bytes,record.end);end=record.end;
            if let Some(reference)=&reference {assert_eq!(&frame.reference,reference);} else {reference=Some(frame.reference.clone());}
            count+=1;
        }
        assert_eq!(count,1000);assert_eq!(end,raw.len() as u64);assert_eq!(archive.owned_bytes(),disk_bytes(&archive));
        assert_eq!(std::fs::read_dir(&archive.root).unwrap().filter(|entry|entry.as_ref().unwrap().path().extension().is_some_and(|extension|extension=="jsonl")).count(),1);
        let conn=archive.store().unwrap().connect().unwrap();assert_eq!(conn.query_row("SELECT count(*) FROM spool_entries",[],|row|row.get::<_,u64>(0)).unwrap(),1);
        let path: String=conn.query_row("SELECT path FROM spool_entries",[],|row|row.get(0)).unwrap();assert_eq!(std::fs::read_to_string(path).unwrap(),raw);
    }

    #[test]
    fn branch_rotation_keeps_unread_frames_and_pinned_originals_in_their_native_scopes() {
        let temp=super::super::tests::Temp::new();let archive=archive(&temp,4*1024*1024);let cancel=AtomicBool::new(false);
        let raw=b"{\"text\":\"Parent original\"}\n{\"text\":\"Child original\"}\n";
        let mut receiver=Receiver::new(raw.as_slice(),archive.clone(),"scope",9);
        let (parent,record)=receiver.next(&cancel,&mut |_|Ok(()),&mut |_|Ok(())).unwrap().unwrap();
        archive.bind(&parent,crate::models::SessionKind::Codex,"scope",9,"parent").unwrap();receiver.rotate().unwrap();
        let (child,next)=receiver.next(&cancel,&mut |_|Ok(()),&mut |_|Ok(())).unwrap().unwrap();
        archive.bind(&child,crate::models::SessionKind::Codex,"scope",9,"child").unwrap();
        assert_ne!(parent.reference,child.reference);assert_eq!(record.start,0);assert_eq!(next.start,0);assert_eq!(next.value["text"],"Child original");
        let mut bytes=Vec::new();write_raw_field(&parent,record.fields.iter().find(|field|field.path=="/text").unwrap(),&mut bytes,&cancel).unwrap();
        assert_eq!(serde_json::from_slice::<serde_json::Value>(&bytes).unwrap(),json!("Parent original"));
        assert!(archive.bind(&parent,crate::models::SessionKind::Codex,"scope",9,"child").unwrap_err().starts_with("detail_unavailable:"));
        assert_eq!(std::fs::read(&parent.path).unwrap(),raw[..parent.bytes as usize]);assert_eq!(archive.owned_bytes(),disk_bytes(&archive));
    }

    #[test]
    fn outgoing_image_originals_stream_past_display_page_budget_without_an_extra_record_buffer() {
        let temp=super::super::tests::Temp::new();let archive=archive(&temp,8*1024*1024);let cancel=AtomicBool::new(false);
        let image="YQ==".repeat(300_000);let value=json!({"type":"dispatch","id":"with-image","text":"Original input","images":[{"mimeType":"image/png","data":image}]});
        let mut journal=Journal::new(archive.clone(),"scope",10).unwrap();let frame=journal.append("dispatch:with-image",&value,&cancel).unwrap();
        assert!(frame.bytes>super::super::limits::PAGE_BYTES as u64);assert!(journal.recording.pending.len()<=INPUT_BYTES);
        let restored:serde_json::Value=serde_json::from_slice(&std::fs::read(&frame.path).unwrap()).unwrap();assert_eq!(restored,value);
        assert_eq!(archive.owned_bytes(),disk_bytes(&archive));
    }

    #[test]
    fn raw_permission_arguments_keep_late_scalars_escapes_and_pinned_prefixes() {
        let temp=super::super::tests::Temp::new();let archive=archive(&temp,8*1024*1024);
        let input=json!({"content":"雪\\\"😀\n".repeat(100_000),"lateNumber":918,"lateFlag":true});
        let raw=format!("{}\n{{\"method\":\"turn/completed\"}}\n",json!({"request":{"input":input},"empty":""}));
        let mut receiver=Receiver::new(raw.as_bytes(),archive,"session",10);let cancel=AtomicBool::new(false);
        let (frame,record)=receiver.next(&cancel,&mut |_|Ok(()),&mut |_|Ok(())).unwrap().unwrap();
        let field=record.fields.iter().find(|field|field.path=="/request/input").unwrap();
        assert!(field.end-field.start>INPUT_BYTES as u64);
        receiver.next(&cancel,&mut |_|Ok(()),&mut |_|Ok(())).unwrap().unwrap();
        let mut bytes=Vec::new();let written=write_raw_field(&frame,field,&mut bytes,&cancel).unwrap();
        assert_eq!(written,bytes.len() as u64);assert_eq!(serde_json::from_slice::<serde_json::Value>(&bytes).unwrap(),input);
        let text=record.fields.iter().find(|field|field.path=="/request/input/content").unwrap();
        bytes.clear();write_raw_field(&frame,text,&mut bytes,&cancel).unwrap();assert_eq!(serde_json::from_slice::<serde_json::Value>(&bytes).unwrap(),input["content"]);
        let empty=record.fields.iter().find(|field|field.path=="/empty").unwrap();
        cancel.store(true,Ordering::Relaxed);bytes.clear();assert_eq!(write_raw_field(&frame,empty,&mut bytes,&cancel).unwrap_err(),"read_canceled");assert!(bytes.is_empty());
        cancel.store(false,Ordering::Relaxed);
        let mut changed=OpenOptions::new().write(true).open(&frame.path).unwrap();changed.seek(SeekFrom::Start(field.start)).unwrap();changed.write_all(b"[").unwrap();
        assert!(write_raw_field(&frame,field,&mut bytes,&cancel).unwrap_err().starts_with("source_changed:"));assert!(bytes.is_empty());
    }

    #[test]
    fn verification_write_failure_retries_only_the_unsaved_suffix() {
        let temp=super::super::tests::Temp::new();let archive=archive(&temp,4*1024*1024);
        let mut recording=archive.begin("session",11).unwrap();let prefix=vec![b'a';100];recording.append(&prefix).unwrap();
        recording.blocks=File::open(&recording.frame.block_path).unwrap();
        let next=vec![b'b';INPUT_BYTES];assert!(recording.append(&next).unwrap_err().starts_with("storage_pressure:"));
        let written=recording.frame.bytes-prefix.len() as u64;assert_eq!(written,INPUT_BYTES as u64-100);assert_eq!(recording.pending.len(),INPUT_BYTES);
        recording.blocks=OpenOptions::new().write(true).open(&recording.frame.block_path).unwrap();
        recording.append(&next[written as usize..]).unwrap();let frame=recording.finish(true).unwrap();
        let mut expected=prefix;expected.extend_from_slice(&next);assert_eq!(std::fs::read(&frame.path).unwrap(),expected);
        assert_eq!(archive.owned_bytes(),disk_bytes(&archive));assert_eq!(std::fs::read(&frame.block_path).unwrap(),Sha256::digest(&expected[..INPUT_BYTES]).to_vec());
        assert_eq!(frame.tail_hash,Sha256::digest(&expected[INPUT_BYTES..]).to_vec());
    }

    #[test]
    fn a_blocked_derived_cache_does_not_prevent_original_persistence_or_control_reception() {
        let temp=super::super::tests::Temp::new();std::fs::write(temp.0.join("chat-index"),b"Unavailable derived cache directory").unwrap();
        let archive=Archive::at(&temp.0).unwrap();let raw=format!("{}\n{{\"method\":\"turn/completed\"}}\n",json!({"input":{"content":"Original 😀\n".repeat(50_000),"late":719}}));
        let mut receiver=Receiver::new(raw.as_bytes(),archive.clone(),"scope",1);let cancel=AtomicBool::new(false);
        let (frame,record)=receiver.next(&cancel,&mut |_|Ok(()),&mut |_|panic!("Unique original storage is available")).unwrap().unwrap();assert!(frame.complete);
        let original=record.fields.iter().find(|field|field.path=="/input").unwrap();let mut bytes=Vec::new();write_raw_field(&frame,original,&mut bytes,&cancel).unwrap();
        assert_eq!(serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()["late"],719);
        let (last,control)=receiver.next(&cancel,&mut |_|Ok(()),&mut |_|Ok(())).unwrap().unwrap();assert_eq!(control.value["method"],"turn/completed");assert!(last.complete);
        assert_eq!(std::fs::read_to_string(last.path).unwrap(),raw);assert_eq!(archive.owned_bytes(),disk_bytes(&archive));
        assert_eq!(std::fs::read(temp.0.join("chat-index")).unwrap(),b"Unavailable derived cache directory");
    }

    #[test]
    fn outgoing_originals_precede_delivery_and_cannot_be_published_as_provider_stdout() {
        let temp=super::super::tests::Temp::new();let archive=archive(&temp,8*1024*1024);let cancel=AtomicBool::new(false);
        let text="雪😀 exact outgoing\n".repeat(10_000);let proposal=json!({"type":"proposal","id":"msg-original","text":text,"images":[],"delivery":"pending"});
        let mut journal=Journal::new(archive.clone(),"scope",10).unwrap();let frame=journal.append("proposal:msg-original",&proposal,&cancel).unwrap();
        assert_eq!(serde_json::from_slice::<serde_json::Value>(&std::fs::read(&frame.path).unwrap()).unwrap(),proposal);
        let owner=read_identity(&archive.root.join(format!("{}.meta.json",frame.reference))).unwrap();assert_eq!(owner["direction"],"outgoing");
        assert_eq!(owner["sessionId"],"scope");assert_eq!(owner["processEpoch"],10);
        assert!(archive.publish(&frame,crate::models::SessionKind::Codex,"scope",10,"native").err().unwrap().starts_with("detail_unavailable:"));
        journal.append("delivery:msg-original",&json!({"type":"delivery","id":"msg-original","state":"written"}),&cancel).unwrap();
        let raw=std::fs::read_to_string(&frame.path).unwrap();let rows=raw.lines().map(|line|serde_json::from_str::<serde_json::Value>(line).unwrap()).collect::<Vec<_>>();
        assert_eq!(rows.len(),2);assert_eq!(rows[0]["text"],text);assert_eq!(rows[1]["state"],"written");
        assert_eq!(archive.owned_bytes(),disk_bytes(&archive));assert!(!temp.0.join("chat-index").exists());
        let store=archive.store().unwrap();let conn=store.connect().unwrap();let row:(String,String)=conn.query_row("SELECT session_scope,reason FROM spool_entries WHERE ref=?1",[frame.reference],|row|Ok((row.get(0)?,row.get(1)?))).unwrap();
        assert_eq!(row.0,"scope");assert_eq!(row.1,"recovered_unique_original");
    }

    #[test]
    fn outgoing_storage_failure_retains_an_exact_retry_without_duplicate_or_new_submission_bytes() {
        let temp=super::super::tests::Temp::new();let archive=archive(&temp,8*1024*1024);let cancel=AtomicBool::new(false);
        let mut journal=Journal::new(archive.clone(),"scope",11).unwrap();let first=json!({"id":"first","text":"Saved original"});
        let frame=journal.append("first",&first,&cancel).unwrap();let first_end=frame.bytes;
        journal.recording.blocks=File::open(&journal.recording.frame.block_path).unwrap();
        let second=json!({"id":"second","text":"x".repeat(300_000),"late":719});
        assert!(journal.append("second",&second,&cancel).err().unwrap().starts_with("storage_pressure:"));
        let saved=journal.recording.frame.bytes;assert!(saved>first_end);assert!(journal.pending.is_some());
        assert!(journal.append("third",&json!({"id":"third","text":"Never sent"}),&cancel).err().unwrap().starts_with("storage_pressure:"));
        assert_eq!(journal.recording.frame.bytes,saved);
        journal.recording.blocks=OpenOptions::new().write(true).open(&journal.recording.frame.block_path).unwrap();
        journal.append("second",&second,&cancel).unwrap();assert!(journal.pending.is_none());
        let raw=std::fs::read_to_string(&frame.path).unwrap();let rows=raw.lines().map(|line|serde_json::from_str::<serde_json::Value>(line).unwrap()).collect::<Vec<_>>();
        assert_eq!(rows,vec![first,second]);assert_eq!(archive.owned_bytes(),disk_bytes(&archive));
        let before=journal.recording.frame.bytes;
        let mut signature=JournalSignature {hash:Sha256::new(),bytes:OUTGOING_RECORD_BYTES-1};
        assert!(signature.write_all(b"x").is_ok());assert!(signature.write_all(b"x").is_err());
        cancel.store(true,Ordering::Relaxed);assert_eq!(journal.append("canceled",&json!({"text":"Canceled"}),&cancel).err().unwrap(),"read_canceled");
        assert_eq!(journal.recording.frame.bytes,before);
    }

}

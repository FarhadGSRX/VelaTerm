//! Field bodies read back from the original bytes, in bounded verified pieces.
//!
//! The index keeps where a field's raw bytes are, never its body. A string longer than one piece is
//! split at the checkpoints the projector reports for it; those are found by parsing the field's
//! record again, or, for a document nested in a string, by parsing the decoded outer string.

use std::io::Read;
use std::sync::Arc;

use sha2::{Digest,Sha256};

use super::limits::*;
use super::projection::{decode_checkpoint,Checkpoint,Projector};
use super::store::{self,FieldMeta,RegisteredSource};

/// Bytes of the raw range of a field: of the source, or decoded from its parent string.
fn raw(source:&RegisteredSource,field:&FieldMeta,start:u64,end:u64,depth:usize) -> Result<Vec<u8>,String> {
    match &field.parent {
        Some(parent)=> {
            let mut reader=DecodedFieldReader::new(source,parent,start,Some(end),depth+1)?;
            let mut bytes=vec![0;(end-start) as usize];reader.read_exact(&mut bytes).map_err(|e|e.to_string())?;Ok(bytes)
        },
        // A field carrying the hash of exactly its own bytes is checked against that hash alone, so an
        // edit elsewhere in its block does not make it unreadable.
        None if field.hash.is_some() && start==field.raw_start && end==field.raw_end=>store::original_range(source,start,end),
        None=>store::verified_range(source,start,end),
    }
}

/// The checkpoints of a string field longer than one piece; empty for one that fits in one.
pub(super) fn pieces(source:&RegisteredSource,field:&FieldMeta,depth:usize) -> Result<Arc<Vec<Checkpoint>>,String> {
    if field.encoding!="string" || field.hash.is_some() {return Ok(Arc::new(Vec::new()));}
    if depth>2 {return Err("unsupported_record: nested detail encoding is too deep".into());}
    let key=(source.id.clone(),source.generation.clone(),field.record_start,match &field.parent {
        Some(parent)=>format!("{}\n{}",parent.path,field.path),None=>field.path.clone()});
    store::cached_pieces(key,|| {
        let mut found=Vec::new();
        match &field.parent {
            None=> {
                store::reproject(source,field.record_start,&mut |piece| {if piece.path==field.path {found.push(piece);}Ok(())})?;
            },
            Some(parent)=> {
                // Inner paths carry the outer path and `/@json` in front of the document's own path.
                let inner=field.path.strip_prefix(&format!("{}/@json",parent.path)).ok_or("unsupported_record: nested detail path is invalid")?.to_owned();
                let reader=DecodedFieldReader::new(source,parent,0,None,depth+1)?;
                Projector::new(reader,0).long_checkpoints_only().next(&mut |piece| {if piece.path==inner {found.push(piece);}Ok(())})?;
            },
        }
        Ok(found)
    })
}

/// One bounded piece of a field: its text and decoded range, and the field's decoded size.
pub(crate) fn read_piece(source:&RegisteredSource,field:&FieldMeta,sequence:u64,depth:usize) -> Result<(String,u64,u64,u64),String> {
    if depth>2 {return Err("unsupported_record: nested detail encoding is too deep".into());}
    let size=field.decoded_bytes;
    if size==0 && sequence==0 {return Ok((String::new(),0,0,0));}
    if field.encoding=="json" || field.encoding=="slice" {
        const RAW_CHUNK:u64=8*1024;
        let relative=sequence.checked_mul(RAW_CHUNK).ok_or("cursor_expired: invalid JSON detail cursor")?;
        if relative>=size {return Err("cursor_expired: JSON detail cursor is outside its field".into());}
        let from=field.raw_start+relative;
        let to=(from+RAW_CHUNK+4).min(field.raw_end);
        let bytes=raw(source,field,from,to,depth)?;
        // A scalar carries the hash of exactly its own bytes.
        if let Some(expected)=field.hash.as_ref().filter(|_|relative==0 && to==field.raw_end) {
            if Sha256::digest(&bytes).as_slice()!=expected.as_slice() {return Err(store::changed(source));}
        }
        let mut left=0;
        while left<bytes.len() && bytes[left]&0xc0==0x80 {left+=1;}
        let mut right=(RAW_CHUNK as usize).min(bytes.len());
        while right<bytes.len() && bytes[right]&0xc0==0x80 {right+=1;}
        let text=std::str::from_utf8(&bytes[left..right]).map_err(|_|store::changed(source))?.to_owned();
        return Ok((text,relative+left as u64,relative+right as u64,size));
    }
    let pieces=pieces(source,field,depth)?;
    if pieces.is_empty() {
        // A string that fits in one piece has no checkpoint; its own hash verifies its raw range.
        if sequence!=0 || field.raw_end<field.raw_start || field.raw_end-field.raw_start>(CHECKPOINT_BYTES+16) as u64 {
            return Err("cursor_expired: detail checkpoint is unavailable".into());
        }
        let bytes=raw(source,field,field.raw_start,field.raw_end,depth)?;
        if field.hash.as_ref().is_some_and(|expected|Sha256::digest(&bytes).as_slice()!=expected.as_slice()) {return Err(store::changed(source));}
        let text=decode_checkpoint(&bytes)?;
        if text.len() as u64!=size {return Err(store::changed(source));}
        return Ok((text,0,size,size));
    }
    let piece=pieces.get(sequence as usize).ok_or("cursor_expired: detail checkpoint is unavailable")?;
    if piece.raw_start<field.raw_start || piece.raw_end>field.raw_end || piece.decoded_start>=size || piece.decoded_end>size {
        return Err("cursor_expired: detail checkpoint is outside its current field".into());
    }
    if piece.raw_end<piece.raw_start || piece.raw_end-piece.raw_start>(CHECKPOINT_BYTES+16) as u64 {return Err("History checkpoint exceeds its byte budget".into());}
    let bytes=raw(source,field,piece.raw_start,piece.raw_end,depth)?;
    if Sha256::digest(&bytes).as_slice()!=piece.hash.as_slice() {return Err(store::changed(source));}
    Ok((decode_checkpoint(&bytes)?,piece.decoded_start,piece.decoded_end,size))
}

/// The piece holding decoded byte `offset` of a field.
pub(super) fn sequence_at(source:&RegisteredSource,field:&FieldMeta,offset:u64) -> Result<u64,String> {
    if field.encoding=="json" || field.encoding=="slice" {return Ok(offset.saturating_sub(3)/(8*1024));}
    let pieces=pieces(source,field,0)?;
    Ok(pieces.iter().filter(|piece|piece.decoded_start<=offset).map(|piece|piece.sequence).max().unwrap_or(0))
}

/// The whole decoded text of a field, piece by piece.
pub(super) fn walk(source:&RegisteredSource,field:&FieldMeta,cancel:&std::sync::atomic::AtomicBool,sink:&mut impl FnMut(&str)->Result<(),String>) -> Result<(),String> {
    let mut sequence=0;
    loop {
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {return Err("read_canceled".into());}
        let (text,_,end,size)=read_piece(source,field,sequence,0)?;sink(&text)?;
        if end>=size {return Ok(());}sequence+=1;
    }
}

/// Read decoded bytes `[start, end)` of a string field through its verified pieces.
pub(crate) struct DecodedFieldReader<'a> {
    source:&'a RegisteredSource,
    field:FieldMeta,
    sequence:u64,
    start:u64,
    end:Option<u64>,
    depth:usize,
    current:std::io::Cursor<Vec<u8>>,
    done:bool,
}

impl<'a> DecodedFieldReader<'a> {
    pub(crate) fn new(source:&'a RegisteredSource,field:&FieldMeta,start:u64,end:Option<u64>,depth:usize) -> Result<Self,String> {
        let sequence=if field.encoding=="string" {
            pieces(source,field,depth)?.iter().filter(|piece|piece.decoded_start<=start).map(|piece|piece.sequence).max().unwrap_or(0)
        } else {0};
        Ok(Self {source,field:field.clone(),sequence,start,end,depth,current:std::io::Cursor::new(Vec::new()),done:false})
    }
}

impl Read for DecodedFieldReader<'_> {
    fn read(&mut self,output:&mut [u8]) -> std::io::Result<usize> {
        if output.is_empty() {return Ok(0);}
        loop {
            let count=self.current.read(output)?;
            if count>0 || self.done {return Ok(count);}
            let (text,start,end,size)=read_piece(self.source,&self.field,self.sequence,self.depth).map_err(std::io::Error::other)?;
            self.sequence+=1;
            let from=self.start.saturating_sub(start) as usize;
            let to=self.end.unwrap_or(end).min(end).saturating_sub(start) as usize;
            if from>to || to>text.len() {return Err(std::io::Error::other("cursor_expired: invalid decoded range"));}
            self.done=end>=size || self.end.is_some_and(|limit|end>=limit);
            self.current=std::io::Cursor::new(text.as_bytes()[from..to].to_vec());
        }
    }
}

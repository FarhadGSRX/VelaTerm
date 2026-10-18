//! Random-access logical prose over verified original fields. Search routing can
//! revisit a word window without copying an entire message into a scratch body.

use std::io::{self,Read,Seek,SeekFrom};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool,Ordering};
use rusqlite::{params,OptionalExtension};
use serde_json::{json,Value};
use super::{api,prose::Prose,store::sql};

#[derive(Clone)]
pub struct Reader {prose:Rc<Prose>,number:u64,position:u64,pub length:u64,cancel:Arc<AtomicBool>}
fn canceled(cancel:&AtomicBool) -> Result<(),String> {if cancel.load(Ordering::Relaxed) {Err("read_canceled".into())} else {Ok(())}}

struct Builder<'a> {prose:&'a Prose,number:u64,sequence:u64,offset:u64,cancel:&'a AtomicBool}
impl Builder<'_> {
    fn add(&mut self,length:u64,descriptor:Value) -> Result<(),String> {
        canceled(self.cancel)?;if length==0 {return Ok(());}let end=self.offset.checked_add(length).ok_or("unsupported_record: logical prose length overflow")?;
        let descriptor=descriptor.to_string();if descriptor.len()>1024*1024 {return Err("unsupported_record: prose descriptor exceeds its metadata budget".into());}
        self.prose.cursor.conn.execute("INSERT INTO prose_byte_segments VALUES(?1,?2,?3,?4,?5)",params![self.number,self.sequence,self.offset,end,descriptor]).map_err(sql)?;
        self.offset=end;self.sequence+=1;Ok(())
    }
    fn literal(&mut self,source:usize,text:&str,verify:Value) -> Result<(),String> {self.add(text.len() as u64,json!({"source":source,"literal":text,"verify":verify}))}
    fn field(&mut self,source:usize,reference:&str,begin:u64,end:u64,proofs:Option<&Value>) -> Result<(),String> {
        if end<begin {return Err("index_incomplete: invalid prose field boundaries".into());}
        self.add(end-begin,json!({"source":source,"reference":reference,"offset":begin,"proofs":proofs}))
    }
    fn size(&self,source:usize,reference:&str) -> Result<u64,String> {
        let (owner,_,_,size,_)=self.prose.cursor.branch.authorize(reference)?;
        let expected=self.prose.cursor.branch.sources.get(source).ok_or("source_changed: prose source is unavailable")?;
        if owner.id!=expected.id || owner.generation!=expected.generation {return Err("source_changed: prose field changed ownership".into());}Ok(size)
    }
    fn native_user(&mut self,source:usize,message:&str) -> Result<(),String> {
        let owner=self.prose.cursor.branch.sources.get(source).ok_or("source_changed: native prose source is unavailable")?.clone();
        let conn=self.prose.cursor.branch.store.connect()?;let mut any=false;
        let shell=super::native_database::user_parts(&conn,&owner,message,|part,reference| {
            canceled(self.cancel)?;if any {self.literal(source,"\n",json!([]))?;}any=true;
            if part["type"]=="text" {
                let reference=reference.ok_or("index_incomplete: original user text is unavailable")?;self.field(source,reference,0,self.size(source,reference)?,None)?;
            } else if part["mime"].as_str().is_some_and(|mime|mime.starts_with("image/")) {
                self.literal(source,"[image]",reference.map(|reference|json!([reference])).unwrap_or(json!([])))?;
            } else {
                self.literal(source,"[file: ",json!([]))?;
                if let Some(reference)=reference {self.field(source,reference,0,self.size(source,reference)?,None)?;}
                self.literal(source,"]",json!([]))?;
            }Ok(())
        })?;
        if !any && shell {if let Some((_,reference))=super::native_database::shell_command(&conn,&owner,message)? {
            self.literal(source,"!",json!([]))?;self.field(source,&reference,0,self.size(source,&reference)?,None)?;
        }}Ok(())
    }
    fn fragment(&mut self,value:&Value) -> Result<(),String> {
        let source=value["source"].as_u64().ok_or("Invalid prose source")? as usize;
        if let Some(message)=value["nativeUser"].as_str() {return self.native_user(source,message);}
        let references=value["references"].as_array().ok_or("Invalid prose references")?;
        if let Some(literal)=value["literal"].as_str() {
            return self.add(literal.len() as u64,json!({"source":source,"literal":literal,"verify":references,"proofs":value["proofs"]}));
        }
        let separator=match value["separator"].as_str() {Some("")=>"",Some("\n\n")=>"\n\n",_=>"\n"};
        let mut first=None;let mut last=0;let mut total=0u64;
        let trim=value["trim"]==true;
        if trim {
            let conn=self.prose.cursor.branch.store.connect()?;
            let owner=self.prose.cursor.branch.sources.get(source).ok_or("source_changed: prose source is unavailable")?;
            for (index,reference) in references.iter().enumerate() {
                canceled(self.cancel)?;let reference=reference.as_str().ok_or("Invalid prose reference")?;
                if index>0 {total+=separator.len() as u64;}
                self.size(source,reference)?;let mut sequence=0;
                loop {
                    canceled(self.cancel)?;let (text,_,end,size)=api::read_piece(&conn,owner,reference,sequence,0)?;
                    for (at,ch) in text.char_indices() {if !ch.is_whitespace() {first.get_or_insert(total+at as u64);last=total+(at+ch.len_utf8()) as u64;}}
                    total+=text.len() as u64;if end>=size {break;}sequence+=1;
                }
            }
        }
        let begin=if trim {first.unwrap_or(0)} else {0};let end=if trim {last} else {u64::MAX};let mut at=0u64;
        for (index,reference) in references.iter().enumerate() {
            let reference=reference.as_str().ok_or("Invalid prose reference")?;
            if index>0 {
                let next=at+separator.len() as u64;let from=begin.saturating_sub(at).min(separator.len() as u64) as usize;let to=end.saturating_sub(at).min(separator.len() as u64) as usize;
                if to>from {self.literal(source,&separator[from..to],json!([]))?;}at=next;
            }
            let size=self.size(source,reference)?;let from=begin.saturating_sub(at).min(size);let to=end.saturating_sub(at).min(size);
            if to>from {self.field(source,reference,from,to,Some(&value["proofs"]))?;}at=at.checked_add(size).ok_or("unsupported_record: logical prose length overflow")?;
        }Ok(())
    }
}

impl Reader {
    pub fn new(prose:Rc<Prose>,number:u64,cancel:Arc<AtomicBool>) -> Result<Self,String> {
        canceled(&cancel)?;if number>=prose.total {return Err("detail_unavailable: logical message is unavailable".into());}
        prose.cursor.conn.execute_batch("CREATE TEMP TABLE IF NOT EXISTS prose_byte_segments(message INTEGER,sequence INTEGER,start INTEGER,end INTEGER,descriptor TEXT,
            PRIMARY KEY(message,sequence));CREATE INDEX IF NOT EXISTS prose_byte_segment_position ON prose_byte_segments(message,start);
            CREATE TEMP TABLE IF NOT EXISTS prose_byte_lengths(message INTEGER PRIMARY KEY,bytes INTEGER);").map_err(sql)?;
        let length=prose.cursor.conn.query_row("SELECT bytes FROM prose_byte_lengths WHERE message=?1",[number],|row|row.get::<_,u64>(0)).optional().map_err(sql)?;
        let length=if let Some(length)=length {length} else {
            prose.cursor.conn.execute("DELETE FROM prose_byte_segments WHERE message=?1",[number]).map_err(sql)?;
            let mut builder=Builder {prose:&prose,number,sequence:0,offset:0,cancel:&cancel};
            let mut statement=prose.cursor.conn.prepare("SELECT descriptor FROM prose_fragments WHERE message=?1 ORDER BY sequence").map_err(sql)?;
            let values=statement.query_map([number],|row|row.get::<_,String>(0)).map_err(sql)?;
            for (index,value) in values.enumerate() {
                let value:Value=serde_json::from_str(&value.map_err(sql)?).map_err(|_|"Invalid prose descriptor")?;
                if index>0 && value["join"]!="" {builder.literal(0,"\n\n",json!([]))?;}builder.fragment(&value)?;
            }
            prose.cursor.conn.execute("INSERT INTO prose_byte_lengths VALUES(?1,?2)",params![number,builder.offset]).map_err(sql)?;builder.offset
        };
        Ok(Self {prose,number,position:0,length,cancel})
    }
}
impl Seek for Reader {
    fn seek(&mut self,position:SeekFrom) -> io::Result<u64> {
        let position=match position {SeekFrom::Start(position)=>position as i128,SeekFrom::Current(delta)=>self.position as i128+delta as i128,
            SeekFrom::End(delta)=>self.length as i128+delta as i128};
        self.position=u64::try_from(position).map_err(|_|io::Error::other("Invalid logical prose position"))?;Ok(self.position)
    }
}
impl Read for Reader {
    fn read(&mut self,output:&mut [u8]) -> io::Result<usize> {
        canceled(&self.cancel).map_err(io::Error::other)?;if output.is_empty() || self.position>=self.length {return Ok(0);}
        let (start,end,raw):(u64,u64,String)=self.prose.cursor.conn.query_row("SELECT start,end,descriptor FROM prose_byte_segments
            WHERE message=?1 AND start<=?2 AND end>?2 ORDER BY start DESC LIMIT 1",params![self.number,self.position],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?)))
            .map_err(|_|io::Error::other("index_incomplete: logical prose byte map is unavailable"))?;
        let value:Value=serde_json::from_str(&raw).map_err(io::Error::other)?;let count=output.len().min((end-self.position) as usize).min(64*1024);
        let source=value["source"].as_u64().and_then(|index|self.prose.cursor.branch.sources.get(index as usize))
            .ok_or_else(||io::Error::other("source_changed: logical prose source is unavailable"))?;
        let conn=self.prose.cursor.branch.store.connect().map_err(io::Error::other)?;
        if let Some(proofs)=value["proofs"].as_array() {for proof in proofs {
            let reference=proof.as_str().ok_or_else(||io::Error::other("Invalid prose proof"))?;
            // Classification evidence is internal and is deliberately absent from
            // user-visible row_fields. It must belong to this selected record.
            let (path,size):(String,u64)=self.prose.cursor.conn.query_row("SELECT f.path,f.decoded_bytes FROM fields f JOIN selected_records r
                ON r.source_id=f.source_id AND r.generation=f.generation AND r.start=f.record_start WHERE f.ref=?1 AND f.source_id=?2 AND f.generation=?3",
                params![reference,source.id,source.generation],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|_|io::Error::other("source_changed: prose proof changed ownership"))?;
            if !super::projection::identity_path(&path) || size>super::limits::MAX_KEY_BYTES as u64 {return Err(io::Error::other("unsupported_record: invalid original prose identity"));}
            let mut proof=api::DecodedFieldReader::new(&conn,source,reference,0,None,0).map_err(io::Error::other)?;let mut bytes=[0;4096];
            loop {canceled(&self.cancel).map_err(io::Error::other)?;if proof.read(&mut bytes)?==0 {break;}}
        }}
        if let Some(text)=value["literal"].as_str() {
            if let Some(references)=value["verify"].as_array() {for reference in references {
                let reference=reference.as_str().ok_or_else(||io::Error::other("Invalid prose reference"))?;
                let (owner,..)=self.prose.cursor.branch.authorize(reference).map_err(io::Error::other)?;
                if owner.id!=source.id || owner.generation!=source.generation {return Err(io::Error::other("source_changed: prose reference changed ownership"));}
                let mut reader=api::DecodedFieldReader::new(&conn,source,reference,0,None,0).map_err(io::Error::other)?;
                let mut bytes=[0;32*1024];loop {canceled(&self.cancel).map_err(io::Error::other)?;if reader.read(&mut bytes)?==0 {break;}}
            }}
            let offset=(self.position-start) as usize;if offset+count>text.len() {return Err(io::Error::other("index_incomplete: logical prose literal changed"));}
            output[..count].copy_from_slice(&text.as_bytes()[offset..offset+count]);
        } else {
            let reference=value["reference"].as_str().ok_or_else(||io::Error::other("Invalid prose reference"))?;
            self.prose.cursor.branch.authorize(reference).map_err(io::Error::other)?;
            let offset=value["offset"].as_u64().ok_or_else(||io::Error::other("Invalid prose field offset"))?+self.position-start;
            let mut reader=api::DecodedFieldReader::new(&conn,source,reference,offset,Some(offset+count as u64),0).map_err(io::Error::other)?;
            reader.read_exact(&mut output[..count])?;
        }
        self.position+=count as u64;Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn original_prose_reader_seeks_across_trim_fragments_utf8_and_independent_cursors() {
        let temp=super::super::tests::Temp::new();let body=format!(" \t{} \n","中文a😀\\\"\n".repeat(30000));
        let raw=[json!({"type":"response_item","payload":{"type":"message","id":"one","role":"assistant","content":[{"type":"output_text","text":body}]}}).to_string(),
            json!({"type":"response_item","payload":{"type":"message","id":"two","role":"assistant","content":[{"type":"output_text","text":"Tail"}]}}).to_string()].join("\n")+"\n";
        let branch=super::super::tests::indexed(&temp,&raw);let cancel=Arc::new(AtomicBool::new(false));let prose=Rc::new(Prose::new(branch,&cancel).unwrap());
        assert_eq!(prose.total,1);let mut expected=Vec::new();prose.body(0,&cancel,&mut expected).unwrap();
        let mut reader=Reader::new(prose,0,cancel.clone()).unwrap();assert_eq!(reader.length,expected.len() as u64);
        for position in [0,1,32767,65535,expected.len()-7,expected.len()-1] {
            reader.seek(SeekFrom::Start(position as u64)).unwrap();let mut fork=reader.clone();let mut actual=[0u8;19];let count=reader.read(&mut actual).unwrap();
            assert_eq!(&actual[..count],&expected[position..position+count]);let mut other=[0u8;3];let count=fork.read(&mut other).unwrap();
            assert_eq!(&other[..count],&expected[position..position+count]);
        }
        reader.seek(SeekFrom::Start(0)).unwrap();let mut actual=Vec::new();reader.read_to_end(&mut actual).unwrap();assert_eq!(actual,expected);
        cancel.store(true,Ordering::Relaxed);assert_eq!(reader.read(&mut [0;1]).unwrap_err().to_string(),"read_canceled");
    }
    #[test]
    fn original_prose_reader_rejects_same_length_changes_in_cached_literals() {
        let temp=super::super::tests::Temp::new();let raw="{\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"id\":\"one\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"Original\"}]}}\n";
        let branch=super::super::tests::indexed(&temp,raw);let path=branch.sources[0].segment.path.clone();let cancel=Arc::new(AtomicBool::new(false));
        let prose=Rc::new(Prose::new(branch,&cancel).unwrap());let mut reader=Reader::new(prose,0,cancel).unwrap();
        std::fs::write(&path,raw.replace("Original","Replaced")).unwrap();
        assert!(reader.read(&mut [0;8]).unwrap_err().to_string().contains("source_changed"));
    }
    #[test]
    fn late_role_proofs_are_checked_without_replaying_the_ignored_giant_field() {
        let temp=super::super::tests::Temp::new();let raw=format!("{{\"type\":\"response_item\",\"payload\":{{\"type\":\"message\",\"id\":\"one\",\"content\":[{{\"type\":\"input_text\",\"text\":{}}}],\"ignored\":{},\"role\":\"user\"}}}}\n",
            json!("body ".repeat(3000)),json!("x".repeat(400000)));
        let branch=super::super::tests::indexed(&temp,&raw);let path=branch.sources[0].segment.path.clone();let cancel=Arc::new(AtomicBool::new(false));
        let prose=Rc::new(Prose::new(branch.clone(),&cancel).unwrap());let mut reader=Reader::new(prose,0,cancel.clone()).unwrap();
        let mut first=[0;8];assert_eq!(reader.read(&mut first).unwrap(),8);reader.seek(SeekFrom::Start(0)).unwrap();
        std::fs::write(&path,raw.replace("\"role\":\"user\"","\"role\":\"tool\"")).unwrap();
        assert!(reader.read(&mut [0;32]).unwrap_err().to_string().starts_with("source_changed:"));
        assert!(Prose::new(branch,&cancel).err().unwrap().starts_with("source_changed:"));
    }
}

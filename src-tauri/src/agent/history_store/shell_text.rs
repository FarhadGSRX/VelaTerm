//! Recognize shell wrappers through verified text chunks, retaining byte ranges instead of outputs.

use serde_json::Value;
use super::{adapter::Refs,fields,projection::Record,store::{FieldMeta,RegisteredSource}};

const TAGS:[&str;6]=["<bash-input>","</bash-input>","<bash-stdout>","</bash-stdout>","<bash-stderr>","</bash-stderr>"];
struct Scan {size:u64,first:u64,last:u64,head:String,tail:String,tags:Vec<(usize,u64)>,overflow:bool}

fn scan(source:&RegisteredSource,field:&FieldMeta) -> Result<Scan,String> {
    let mut result=Scan {size:0,first:u64::MAX,last:0,head:String::new(),tail:String::new(),tags:Vec::new(),overflow:false};
    let mut sequence=0;let mut carry=String::new();
    loop {
        let (text,from,end,size)=fields::read_piece(source,field,sequence,0)?;
        for (offset,ch) in text.char_indices() {if !ch.is_whitespace() {result.first=result.first.min(from+offset as u64);result.last=from+(offset+ch.len_utf8()) as u64;}}
        if result.head.len()<64 {result.head.push_str(&super::adapter::preview(&text,64-result.head.len()));}
        let base=from.saturating_sub(carry.len() as u64);let previous=carry.len();carry.push_str(&text);
        if !result.overflow {
            let mut tags=Vec::new();
            for (kind,tag) in TAGS.iter().enumerate() {
                for (offset,_) in carry.match_indices(tag) {if offset+tag.len()>previous {tags.push((kind,base+offset as u64));}}
            }
            tags.sort_by_key(|tag|tag.1);
            for tag in tags {if result.tags.len()==7 {result.overflow=true;result.tags.clear();break;}result.tags.push(tag);}
        }
        let mut begin=carry.len().saturating_sub(32);while !carry.is_char_boundary(begin) {begin+=1;}carry.drain(..begin);
        result.tail.push_str(&text);let mut begin=result.tail.len().saturating_sub(2048);while !result.tail.is_char_boundary(begin) {begin+=1;}result.tail.drain(..begin);
        result.size=size;if end>=size {break;}sequence+=1;
    }Ok(result)
}
fn small(source:&RegisteredSource,field:&FieldMeta,from:u64,to:u64) -> Result<String,String> {
    if to<from || to-from>2048 {return Err("unsupported_record: shell wrapper metadata exceeds its byte budget".into());}
    use std::io::Read;
    let mut reader=fields::DecodedFieldReader::new(source,field,from,Some(to),0)?;
    let mut bytes=vec![0;(to-from) as usize];reader.read_exact(&mut bytes).map_err(|e|e.to_string())?;
    String::from_utf8(bytes).map_err(|_|"source_changed: shell wrapper boundary is invalid".into())
}

/// The record's only message text field, when it has exactly one.
pub(super) fn single(record:&Record,refs:&Refs) -> Option<FieldMeta> {
    let mut fields=record.fields.iter().filter(|field|field.encoding=="string" && (field.path=="/message/content"
        || field.path.starts_with("/message/content/") && field.path.ends_with("/text"))).collect::<Vec<_>>();
    fields.sort_by_key(|field|field.start);
    let mut found=None;
    for field in fields {
        if !record.value.pointer(&field.path).is_some_and(Value::is_string) {continue;}
        if found.is_some() {return None;}found=refs.get(&field.path).map(|(_,meta)|meta.clone());
    }
    found
}

/// Whether the field is exactly one `<bash-input>` wrapper.
pub(super) fn input(source:&RegisteredSource,field:&FieldMeta) -> Result<bool,String> {
    let scanned=scan(source,field)?;let begin=scanned.first;let end=scanned.last;
    if end<begin.saturating_add((TAGS[0].len()+TAGS[1].len()) as u64) {return Ok(false);}
    Ok(small(source,field,begin,begin+TAGS[0].len() as u64)?==TAGS[0] && small(source,field,end-TAGS[1].len() as u64,end)?==TAGS[1])
}
/// The decoded ranges of standard output, and of standard error when present, of a field that is
/// exactly one `<bash-stdout>` wrapper, optionally followed by one `<bash-stderr>` wrapper.
pub(super) fn output(source:&RegisteredSource,field:&FieldMeta) -> Result<Option<((u64,u64),Option<(u64,u64)>)>,String> {
    let scanned=scan(source,field)?;
    if scanned.overflow {return Ok(None);}
    let ranges=match scanned.tags.as_slice() {
        [(2,begin),(3,end)] if *begin==scanned.first && *end+TAGS[3].len() as u64==scanned.last=>
            Some(((*begin+TAGS[2].len() as u64,*end),None)),
        [(2,begin),(3,end),(4,error),(5,last)] if *begin==scanned.first && *end+TAGS[3].len() as u64==*error
            && *last+TAGS[5].len() as u64==scanned.last=>Some(((*begin+TAGS[2].len() as u64,*end),Some((*error+TAGS[4].len() as u64,*last)))),
        _=>None,
    };
    Ok(ranges)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn long_output_tags_split_across_chunks_keep_literal_status_and_reject_ambiguity() {
        let body="雪😀 literal exit_code=7\n".repeat(20_000);
        for ambiguous in [false,true] {
            let temp=super::super::tests::Temp::new();
            let text=format!("<bash-stdout>{body}{}</bash-stdout><bash-stderr>warning</bash-stderr>",if ambiguous {"<bash-input>"} else {""});
            let raw=serde_json::json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":text}]}}).to_string()+"\n";
            let branch=super::super::tests::indexed(&temp,&raw);let source=&branch.sources[0];
            let (_,refs)=super::super::event_stream::read_record(source,0).unwrap();let (_,field)=refs["/payload/content/0/text"].clone();
            let layout=output(source,&field).unwrap();assert_eq!(layout.is_none(),ambiguous);
            if let Some((stdout,stderr))=layout {assert_eq!(stdout,(13,13+body.len() as u64));assert_eq!(stderr.unwrap().1-stderr.unwrap().0,7);}
        }
    }
}

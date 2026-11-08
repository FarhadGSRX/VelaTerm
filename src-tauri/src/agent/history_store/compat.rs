//! A bounded compatibility bridge. Large recordings require the paged contract before materialization.

use std::io::Read;
use std::path::Path;

use super::{limits::PAGE_BYTES,projection::Projector,source::Segment};

pub const UPGRADE:&str="detail_requires_paging: this recording requires the paged conversation reader";

pub fn read_path(path:&Path) -> Result<String,String> {
    read_segments(&[Segment {path:path.into(),native_id:String::new(),end_byte:None,end_ordinal:None}])
}

pub fn read_segments(segments:&[Segment]) -> Result<String,String> {
    let mut content=String::new();
    for segment in segments {
        let file=std::fs::File::open(&segment.path).map_err(|_|"detail_unavailable: conversation recording is unavailable")?;
        let size=file.metadata().map_err(|e|e.to_string())?.len();
        let limit=segment.end_byte.unwrap_or(size);
        if limit>size {return Err("source_changed: conversation recording was truncated".into());}
        let budget=PAGE_BYTES-content.len();
        if segment.end_ordinal.is_none() && limit>budget as u64 {return Err(UPGRADE.into());}
        // Compatibility can materialize only a response whose entire original input was bounded
        // before reading. An ordinal cutoff reads at most the remaining budget plus one sentinel byte.
        let mut raw=Vec::new();file.take(limit.min(budget as u64+1)).read_to_end(&mut raw).map_err(|e|e.to_string())?;
        let limited=(raw.len() as u64)<limit;
        let mut parser=Projector::new(raw.as_slice(),0);let mut cutoff=false;
        while let Some(record)=parser.next(&mut |_|Ok(()))? {
            if segment.end_ordinal.is_some_and(|end|record.value["ordinal"].as_u64().is_some_and(|ordinal|ordinal>=end)) {cutoff=true;break;}
            if !record.complete && limited {return Err(UPGRADE.into());}
            if let Some(error)=record.error {return Err(error);}
            if !record.complete {return Err("index_incomplete: conversation recording has an unfinished final record".into());}
            let bytes=&raw[record.start as usize..record.end as usize];
            if content.len()+bytes.len()>PAGE_BYTES {return Err(UPGRADE.into());}
            content.push_str(std::str::from_utf8(bytes).map_err(|_|"unsupported_record: conversation recording has invalid UTF-8")?);
        }
        if limited && !cutoff {return Err(UPGRADE.into());}
    }
    Ok(content)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor,Write};

    #[test]
    fn legacy_bridge_rejects_a_giant_record_before_building_a_full_value() {
        let temp=super::super::tests::Temp::new();let path=temp.0.join("native.jsonl");
        let mut source=super::super::tests::Giant {prefix:Cursor::new(b"{\"diff\":\"".to_vec()),remaining:40*1024*1024,
            suffix:Cursor::new(b"\",\"type\":\"FileChange\"}\n".to_vec())};
        let mut file=std::fs::File::create(&path).unwrap();std::io::copy(&mut source,&mut file).unwrap();file.flush().unwrap();
        assert_eq!(read_path(&path).unwrap_err(),UPGRADE);
    }

    #[test]
    fn legacy_bridge_keeps_small_values_and_never_reads_past_a_fork_cutoff() {
        let temp=super::super::tests::Temp::new();let path=temp.0.join("native.jsonl");
        let first="{\"ordinal\":1,\"text\":\"雪\\n😀\"}\n";
        std::fs::write(&path,format!("{first}{{\"ordinal\":2,\"text\":\"{}\"}}\n","x".repeat(100_000))).unwrap();
        let segment=Segment {path,native_id:"thread".into(),end_byte:Some(first.len() as u64),end_ordinal:None};
        let result=read_segments(&[segment]).unwrap();assert_eq!(serde_json::from_str::<serde_json::Value>(result.trim()).unwrap()["text"],"雪\n😀");
    }
}

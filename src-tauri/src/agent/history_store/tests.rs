use std::io::{Cursor, Read, Write};

use super::limits::*;
use super::projection::{decode_checkpoint, Checkpoint, Projector};

struct Fragmented<R> { inner: R, bytes: usize }
impl<R: Read> Read for Fragmented<R> {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        let len = bytes.len().min(self.bytes);
        self.inner.read(&mut bytes[..len])
    }
}

#[test]
fn document_history_arrays_apply_budgets_per_element_and_keep_late_metadata() {
    struct Repeated {prefix:Cursor<Vec<u8>>,row:Cursor<Vec<u8>>,remaining:usize,suffix:Cursor<Vec<u8>>}
    impl Read for Repeated {
        fn read(&mut self,output:&mut [u8]) -> std::io::Result<usize> {
            let count=self.prefix.read(output)?;if count>0 {return Ok(count);}
            while self.remaining>0 {
                let count=self.row.read(output)?;if count>0 {return Ok(count);}
                self.remaining-=1;if self.remaining>0 {self.row.set_position(0);}
            }self.suffix.read(output)
        }
    }
    let source=Repeated {prefix:Cursor::new(b"{\"cwd\":\"/repo\",\"history\":[{}".to_vec()),
        row:Cursor::new(format!(",{{\"user\":{{\"content\":{{\"Prompt\":{{\"prompt\":\"{}\"}}}}}},\"assistant\":{{\"Response\":{{\"content\":\"Reply\",\"message_id\":\"native\"}}}}}}","x".repeat(PREVIEW_BYTES+1)).into_bytes()),
        remaining:5000,suffix:Cursor::new(b"],\"conversation_id\":\"selected\"}".to_vec())};
    let mut parser=Projector::document(source,0);let mut count=0;let mut checkpoints=0;
    let header=parser.split_object_array("history",&mut |checkpoint| {
        assert!(checkpoint.raw_end-checkpoint.raw_start<=(CHECKPOINT_BYTES+16) as u64);checkpoints+=1;Ok(())
    },&mut |record| {
        count+=1;assert!(record.complete);assert!(record.error.is_none());assert!(record.fields.len()<20);
        if count>1 {assert_eq!(record.value["assistant"]["Response"]["message_id"],"native");
            let field=record.fields.iter().find(|field|field.path=="/user/content/Prompt/prompt").unwrap();
            assert!(field.truncated);assert_eq!(field.decoded_bytes,(PREVIEW_BYTES+1) as u64);}
        Ok(())
    }).unwrap();
    assert_eq!(count,5001);assert_eq!(header.value["conversation_id"],"selected");assert_eq!(header.value["cwd"],"/repo");
    assert!(header.value["history"].as_array().unwrap().is_empty());assert!(header.fields.len()<5);assert!(checkpoints>5000);
}

#[test]
fn split_document_rejects_duplicate_arrays_trailing_content_and_failed_sinks() {
    for raw in ["{\"history\":[],\"history\":[]}","{\"history\":[]}{}","{\"history\":[{\"a\":1},]}"] {
        assert!(Projector::document(raw.as_bytes(),0).split_object_array("history",&mut |_|Ok(()),&mut |_|Ok(())).is_err());
    }
    let raw="{\"history\":[{\"content\":\"original\"}]}";
    assert_eq!(Projector::document(raw.as_bytes(),0).split_object_array("history",&mut |_|Err("storage_pressure: full".into()),&mut |_|Ok(())).unwrap_err(),"storage_pressure: full");
    assert_eq!(Projector::document(raw.as_bytes(),0).split_object_array("history",&mut |_|Ok(()),&mut |_|Err("read_canceled".into())).unwrap_err(),"read_canceled");
}

#[test]
fn nested_metadata_arrays_keep_root_fields_and_element_checkpoints_separate() {
    let turns=(0..2000).map(|number|serde_json::json!({"message_ids":[format!("message-{number}")],"result":{"Ok":{"content":[{"kind":"text","data":"雪😀\\\""}]}}})).collect::<Vec<_>>();
    let raw=serde_json::json!({"before":"Before","session_state":{"conversation_metadata":{"user_turn_metadatas":turns},"version":"v1"},"session_id":"selected"}).to_string();
    let mut parser=Projector::document(Fragmented {inner:raw.as_bytes(),bytes:7},0);let mut count=0;let mut checkpoints=Vec::new();
    let header=parser.split_document_array("/session_state/conversation_metadata/user_turn_metadatas",&mut |piece|{checkpoints.push(piece);Ok(())},&mut |record| {
        assert!(record.fields.len()<10);assert!(record.start>0);assert_eq!(record.value["message_ids"][0],format!("message-{count}"));count+=1;Ok(())
    }).unwrap();
    assert_eq!(count,2000);assert_eq!(header.value["session_state"]["version"],"v1");assert_eq!(header.value["session_id"],"selected");
    assert!(header.fields.len()<10);
    for checkpoint in checkpoints {assert!(checkpoint.record_start==0 || checkpoint.raw_start>checkpoint.record_start);
        assert!(decode_checkpoint(&raw.as_bytes()[checkpoint.raw_start as usize..checkpoint.raw_end as usize]).is_ok());}
    let duplicate="{\"session_state\":{\"conversation_metadata\":{\"user_turn_metadatas\":[]}},\"session_state\":{}}";
    assert!(Projector::document(duplicate.as_bytes(),0).split_document_array("/session_state/conversation_metadata/user_turn_metadatas",&mut |_|Ok(()),&mut |_|Ok(())).is_err());
}

#[test]
fn projection_matches_json_across_utf8_escapes_and_surrogates() {
    let raw = br#"{"payload":{"text":"a\n\"\\\uD83D\uDE80","arguments":"{\"path\":\"a\\nb\"}"},"type":"response_item","n":-1.25e+2,"b":true,"x":null}"#;
    let expected: serde_json::Value = serde_json::from_slice(raw).unwrap();
    let raw = [raw.as_slice(), b"\n"].concat();
    for bytes in 1..=17 {
        let mut parser = Projector::new(Fragmented { inner: Cursor::new(&raw), bytes }, 0);
        let mut chunks = Vec::new();
        let record = parser.next(&mut |chunk| { chunks.push(chunk); Ok(()) }).unwrap().unwrap();
        assert_eq!(record.error, None);
        assert_eq!(record.value, expected);
        assert!(record.complete);
        for chunk in chunks {
            let text = decode_checkpoint(&raw[chunk.raw_start as usize..chunk.raw_end as usize]).unwrap();
            assert_eq!(text.len() as u64, chunk.decoded_end - chunk.decoded_start);
        }
    }
    let raw = "{\"text\":\"中文Ω🚀\"}\n";
    let mut parser = Projector::new(Fragmented { inner: raw.as_bytes(), bytes: 1 }, 0);
    assert_eq!(parser.next(&mut |_| Ok(())).unwrap().unwrap().value["text"], "中文Ω🚀");
}

/// Generate a large single string without allocating its body in the test process.
pub(super) struct Giant { pub(super) prefix: Cursor<Vec<u8>>, pub(super) remaining: usize, pub(super) suffix: Cursor<Vec<u8>> }
impl Read for Giant {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        let count = self.prefix.read(bytes)?;
        if count > 0 { return Ok(count); }
        if self.remaining > 0 {
            let count = self.remaining.min(bytes.len());
            bytes[..count].fill(b'x'); self.remaining -= count; return Ok(count);
        }
        self.suffix.read(bytes)
    }
}

#[test]
fn forty_megabyte_single_line_retains_only_preview_and_streams_checkpoints() {
    let size = 40 * 1024 * 1024;
    let source = Giant { prefix: Cursor::new(b"{\"diff\":\"".to_vec()), remaining: size,
        suffix: Cursor::new(b"\",\"type\":\"FileChange\"}\n".to_vec()) };
    let mut parser = Projector::new(source, 0);
    let mut count = 0;
    let mut decoded = 0;
    let record = parser.next(&mut |chunk: Checkpoint| {
        assert!(chunk.raw_end - chunk.raw_start <= (CHECKPOINT_BYTES + 12) as u64);
        count += 1; decoded += chunk.decoded_end - chunk.decoded_start; Ok(())
    }).unwrap().unwrap();
    assert_eq!(record.error, None);
    assert_eq!(record.value["type"], "FileChange");
    assert_eq!(record.value["diff"].as_str().unwrap().len(), PREVIEW_BYTES);
    assert_eq!(record.fields[0].decoded_bytes, size as u64);
    assert!(record.fields[0].truncated);
    assert!(count > 1000 && decoded >= size as u64);
}

#[test]
fn bad_middle_record_and_incomplete_tail_keep_physical_coverage() {
    let raw = b"{\"text\":\"ok\"}\n{\"text\":\"bad\\q\"}\n{\"text\":\"next\"}\n{\"text\":\"half";
    let mut parser = Projector::new(&raw[..], 100);
    let first = parser.next(&mut |_| Ok(())).unwrap().unwrap();
    let bad = parser.next(&mut |_| Ok(())).unwrap().unwrap();
    let next = parser.next(&mut |_| Ok(())).unwrap().unwrap();
    let tail = parser.next(&mut |_| Ok(())).unwrap().unwrap();
    assert_eq!(first.start, 100);
    assert!(bad.error.is_some() && bad.complete);
    assert_eq!(bad.end, next.start);
    assert_eq!(next.value["text"], "next");
    assert!(!tail.complete && tail.error.is_some());
}

#[test]
fn excessive_depth_is_explicit_and_never_falls_back_to_full_json() {
    let raw = format!("{}0{}\n{{\"text\":\"after\"}}\n", "[".repeat(MAX_DEPTH + 10), "]".repeat(MAX_DEPTH + 10));
    let mut parser = Projector::new(raw.as_bytes(), 0);
    let bad = parser.next(&mut |_| Ok(())).unwrap().unwrap();
    assert!(bad.error.unwrap().starts_with("unsupported_record:"));
    assert_eq!(parser.next(&mut |_| Ok(())).unwrap().unwrap().value["text"], "after");
}

pub(super) struct Temp(pub(super) std::path::PathBuf);
impl Temp {
    pub(super) fn new() -> Self {
        let path=std::env::temp_dir().join(format!("vlx-history-test-{}",uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&path).unwrap(); Self(path)
    }
}
impl Drop for Temp {fn drop(&mut self) {super::store::release_test_directory(&self.0);std::fs::remove_dir_all(&self.0).unwrap();}}

pub(super) fn indexed(temp:&Temp,raw:&str) -> super::api::Branch {
    indexed_kind(temp,raw,crate::models::SessionKind::Codex)
}
pub(super) fn indexed_kind(temp:&Temp,raw:&str,kind:crate::models::SessionKind) -> super::api::Branch {
    let path=temp.0.join("native.jsonl");
    std::fs::write(&path,raw).unwrap();
    let store=super::store::Store::at(&temp.0).unwrap();
    let source=store.register(kind,super::source::Segment {path,native_id:"thread".into(),end_byte:None,end_ordinal:None}).unwrap();
    store.scan(kind,&source,&std::sync::atomic::AtomicBool::new(false),false).unwrap();
    super::api::Branch {store,sources:vec![source],generation:"branch".into(),kind,native_id:"thread".into(),error:None,scope:"test".into(),parent:None}
}

#[test]
fn sqlite_pages_and_tool_results_are_addressable_without_replay() {
    let temp=Temp::new();
    let mut raw=String::new();
    for index in 0..130 {
        raw.push_str(&serde_json::json!({"type":"response_item","ordinal":index,"payload":{"type":"message","id":format!("u{index}"),
            "role":"user","content":[{"type":"input_text","text":format!("message {index}")}]}}).to_string()); raw.push('\n');
    }
    raw.push_str("{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call\",\"call_id\":\"c\",\"name\":\"shell\",\"arguments\":\"{\\\"command\\\":\\\"echo ok\\\"}\"}}\n");
    raw.push_str("{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call_output\",\"call_id\":\"c\",\"output\":\"ok\"}}\n");
    let branch=indexed(&temp,&raw);
    let recent=branch.page(None,None,None).unwrap();
    assert_eq!(recent["coverage"],"complete");
    assert_eq!(recent["rows"].as_array().unwrap().len(),60);
    assert_eq!(recent["knownRows"],131);
    let first=recent["rows"][0]["id"].as_str().unwrap();
    let older=branch.page(Some(first),None,None).unwrap();
    assert_eq!(older["rows"][0]["text"],"message 11");
    let tool=recent["rows"].as_array().unwrap().last().unwrap();
    assert_eq!(tool["name"],"shell"); assert_eq!(tool["status"],"completed");
    let manifest=branch.manifest(tool["id"].as_str().unwrap(),0).unwrap();
    assert_eq!(manifest["fields"].as_array().unwrap().len(),2);
    let detail=branch.detail(manifest["fields"][1]["detailRef"].as_str().unwrap(),None,0).unwrap();
    assert_eq!(detail["text"],"ok");
    assert!(serde_json::to_vec(&recent).unwrap().len()<=PAGE_BYTES);
    assert!(branch.page(Some("foreign"),None,None).unwrap_err().starts_with("cursor_expired:"));
}

#[test]
fn cold_locators_wait_for_indexing_instead_of_expiring_before_the_row_is_known() {
    use crate::models::SessionKind;
    let temp=Temp::new();let path=temp.0.join("native.jsonl");
    std::fs::write(&path,"{\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"id\":\"user\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"Original message\"}]}}\n").unwrap();
    let store=super::store::Store::at(&temp.0).unwrap();let source=store.register(SessionKind::Codex,super::source::Segment {path,native_id:"thread".into(),end_byte:None,end_ordinal:None}).unwrap();
    let branch=super::api::Branch {store:store.clone(),sources:vec![source.clone()],generation:"branch".into(),kind:SessionKind::Codex,native_id:"thread".into(),error:None,scope:"test".into(),parent:None};
    for (before,after,anchor,key) in [(Some("future"),None,None,"before"),(None,Some("future"),None,"after"),(None,None,Some("future"),"anchor")] {
        let page=branch.page(before,after,anchor).unwrap();assert_eq!(page["indexing"],true);assert_eq!(page["pendingLocation"][key],"future");
        assert!(page["rows"].as_array().unwrap().is_empty());assert_eq!(page["totalRows"],serde_json::Value::Null);
    }
    store.scan(SessionKind::Codex,&source,&std::sync::atomic::AtomicBool::new(false),false).unwrap();
    let page=branch.page(None,None,None).unwrap();assert_eq!(page["indexing"],false);
    let id=page["rows"][0]["id"].as_str().unwrap();assert_eq!(branch.page(None,None,Some(id)).unwrap()["rows"][0]["text"],"Original message");
    assert!(branch.page(None,None,Some("future")).unwrap_err().starts_with("cursor_expired:"));
}

#[test]
fn nested_arguments_keep_verified_layered_details() {
    let temp=Temp::new();
    let inner=serde_json::json!({"path":"recorded.json","old_string":"中\\\"🚀\n".repeat(5000),"new_string":"new"}).to_string();
    let raw=serde_json::json!({"type":"response_item","payload":{"type":"function_call","call_id":"edit","name":"Edit","arguments":inner}}).to_string()+"\n";
    let branch=indexed(&temp,&raw);
    let page=branch.page(None,None,None).unwrap();
    let manifest=branch.manifest(page["rows"][0]["id"].as_str().unwrap(),0).unwrap();
    let field=manifest["fields"].as_array().unwrap().iter().find(|field|field["path"].as_str().unwrap().ends_with("/old_string")).unwrap();
    let reference=field["detailRef"].as_str().unwrap();
    let mut text=String::new(); let mut sequence=0;
    loop {
        let chunk=branch.detail(reference,None,sequence).unwrap();
        assert!(serde_json::to_vec(&chunk).unwrap().len()<=DETAIL_BYTES);
        text.push_str(chunk["text"].as_str().unwrap());
        if chunk["eof"]==true {break;}
        sequence=chunk["nextCursor"].as_u64().unwrap();
    }
    assert_eq!(text,"中\\\"🚀\n".repeat(5000));
}

#[test]
fn source_append_resumes_at_durable_boundary_and_old_details_reject_rewrites() {
    use crate::models::SessionKind;
    let temp=Temp::new();
    let raw="{\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"original\"}]}}\n";
    let mut branch=indexed(&temp,raw);
    let first=branch.page(None,None,None).unwrap();
    let manifest=branch.manifest(first["rows"][0]["id"].as_str().unwrap(),0).unwrap();
    let reference=manifest["fields"][0]["detailRef"].as_str().unwrap();
    let old=branch.sources[0].clone();
    std::fs::OpenOptions::new().append(true).open(&old.segment.path).unwrap().write_all(raw.as_bytes()).unwrap();
    let source=branch.store.register(SessionKind::Codex,old.segment.clone()).unwrap();
    assert_eq!(source.generation,old.generation); assert_eq!(source.indexed,raw.len() as u64);
    branch.store.scan(SessionKind::Codex,&source,&std::sync::atomic::AtomicBool::new(false),false).unwrap();
    branch.sources=vec![source];
    assert_eq!(branch.page(None,None,None).unwrap()["knownRows"],2);
    assert_eq!(branch.detail(reference,None,0).unwrap()["text"],"original");
    let modified=raw.replace("original","modified");
    std::fs::write(&old.segment.path,modified.repeat(2)).unwrap();
    assert!(branch.detail(reference,None,0).unwrap_err().starts_with("source_changed:"));
}

#[test]
fn fork_cutoff_blocks_parent_suffix_rows_and_detail_refs() {
    let temp=Temp::new();
    let line=|id:&str|serde_json::json!({"type":"response_item","payload":{"type":"message","id":id,"role":"user","content":[{"type":"input_text","text":id}]}}).to_string()+"\n";
    let before=line("before");let after=line("after");
    let mut branch=indexed(&temp,&(before.clone()+&after));
    let full=branch.page(None,None,None).unwrap();
    let manifest=branch.manifest(full["rows"][1]["id"].as_str().unwrap(),0).unwrap();
    let reference=manifest["fields"][0]["detailRef"].as_str().unwrap();
    branch.sources[0].segment.end_byte=Some(before.len() as u64);
    assert_eq!(branch.page(None,None,None).unwrap()["knownRows"],1);
    assert!(branch.detail(reference,None,0).unwrap_err().contains("outside the selected history branch"));
}

#[test]
fn classification_after_payload_budget_is_independent_of_field_order() {
    let mut object=serde_json::Map::new();
    for index in 0..40 {object.insert(format!("unknown{index:02}"),serde_json::json!("x".repeat(PREVIEW_BYTES)));}
    // serde_json sorts object keys, so force the classification key to the end physically.
    let mut raw=serde_json::Value::Object(object).to_string(); raw.pop();
    raw.push_str(",\"type\":\"FileChange\",\"id\":\"native-id\"}\n");
    raw=raw.replace("\"type\":\"FileChange\"","\"text\":\"late body\",\"type\":\"FileChange\"");
    let mut parser=Projector::new(raw.as_bytes(),0);
    let record=parser.next(&mut |_|Ok(())).unwrap().unwrap();
    assert_eq!(record.error,None);
    assert_eq!(record.value["type"],"FileChange");
    assert_eq!(record.value["id"],"native-id");
    assert_eq!(record.value["text"],"late body");
    assert!(record.fields.iter().any(|field|field.truncated));
}

#[test]
fn checkpoint_storage_errors_stop_before_draining_a_large_tail() {
    let source=Giant {prefix:Cursor::new(b"{\"diff\":\"".to_vec()),remaining:40*1024*1024,suffix:Cursor::new(b"\"}\n".to_vec())};
    let mut parser=Projector::new(source,0);
    let error=parser.next(&mut |_|Err("Storage pressure".into())).unwrap_err();
    assert_eq!(error,"Storage pressure");
    assert!(parser.offset()<(INPUT_BYTES as u64));
}

#[test]
fn json_container_details_preserve_numbers_structure_and_utf8_boundaries() {
    let temp=Temp::new();
    let changes=serde_json::json!({"file.json":{"kind":{"type":"update","move_path":"renamed.json"},
        "count":123,"enabled":true,"unified_diff":"中文🚀\\\"\n".repeat(5000)}});
    let raw=serde_json::json!({"type":"event_msg","payload":{"type":"item_completed","item":{
        "id":"file-change","type":"FileChange","changes":changes}}}).to_string()+"\n";
    let branch=indexed(&temp,&raw);
    let page=branch.page(None,None,None).unwrap();
    let manifest=branch.manifest(page["rows"][0]["id"].as_str().unwrap(),0).unwrap();
    assert_eq!(manifest["files"][0]["changeType"],"update");
    let field=manifest["fields"].as_array().unwrap().iter().find(|f|f["path"]=="/payload/item/changes").unwrap();
    assert_eq!(field["kind"],"json");
    let mut actual=String::new(); let mut sequence=0;
    loop {
        let chunk=branch.detail(field["detailRef"].as_str().unwrap(),None,sequence).unwrap();
        assert!(serde_json::to_vec(&chunk).unwrap().len()<=DETAIL_BYTES);
        actual.push_str(chunk["text"].as_str().unwrap());
        if chunk["eof"]==true {break;} sequence+=1;
    }
    assert_eq!(serde_json::from_str::<serde_json::Value>(&actual).unwrap(),changes);
    let mut file=std::fs::OpenOptions::new().write(true).open(&branch.sources[0].segment.path).unwrap();
    file.write_all(b" ").unwrap();
    assert!(branch.detail(field["detailRef"].as_str().unwrap(),None,0).unwrap_err().starts_with("source_changed:"));
}

#[test]
fn two_hundred_fifty_six_megabyte_record_does_not_expand_the_projection() {
    let size=256*1024*1024;
    let source=Giant {prefix:Cursor::new(b"{\"unknown\":\"".to_vec()),remaining:size,suffix:Cursor::new(b"\",\"text\":\"kept\",\"type\":\"event_msg\"}\n".to_vec())};
    let mut parser=Projector::new(source,0);
    let mut largest=0;
    let record=parser.next(&mut |chunk| {largest=largest.max(chunk.raw_end-chunk.raw_start);Ok(())}).unwrap().unwrap();
    assert!(record.error.is_none()); assert_eq!(record.value["text"],"kept");
    assert!(serde_json::to_vec(&record.value).unwrap().len()<PREVIEW_BYTES+200);
    assert!(largest<=(CHECKPOINT_BYTES+12) as u64);
}

#[test]
#[ignore = "Requires the explicitly selected read-only native recording"]
fn read_only_native_history_performance_probe() {
    let path=std::path::PathBuf::from(std::env::var_os("VLX_HISTORY_PROBE_SOURCE").expect("VLX_HISTORY_PROBE_SOURCE is required"));
    performance_probe(path);
}

#[test]
#[ignore = "Creates and removes the explicitly selected synthetic performance fixture"]
fn synthetic_history_performance_probe() {
    let temp=Temp::new();
    let path=temp.0.join("synthetic.jsonl");
    let requested=std::env::var("VLX_HISTORY_PROBE_BYTES").unwrap_or_else(|_|"536870912".into()).parse::<usize>().unwrap();
    let mut output=std::fs::File::create(&path).unwrap();
    let prefix=b"{\"type\":\"event_msg\",\"payload\":{\"type\":\"item_completed\",\"item\":{\"id\":\"change\",\"type\":\"FileChange\",\"changes\":{\"large.json\":{\"kind\":\"update\",\"unified_diff\":\"";
    let suffix=b"\"}}}}}\n";
    let mut remaining=requested;
    while remaining>0 {
        let count=remaining.min(40*1024*1024);
        let mut raw=Giant {prefix:Cursor::new(prefix.to_vec()),remaining:count,suffix:Cursor::new(suffix.to_vec())};
        std::io::copy(&mut raw,&mut output).unwrap(); remaining-=count;
    }
    drop(output);
    performance_probe(path);
}

fn performance_probe(path:std::path::PathBuf) {
    use crate::models::SessionKind;
    let temp=Temp::new();
    let mut original=std::fs::File::open(&path).unwrap();
    let before=super::source::stamp(&mut original).unwrap();
    let store=super::store::Store::at(&temp.0).unwrap();
    let source=store.register(SessionKind::Codex,super::source::Segment {path:path.clone(),native_id:"readonly-probe".into(),end_byte:None,end_ordinal:None}).unwrap();
    let branch=super::api::Branch {store:store.clone(),sources:vec![source.clone()],generation:"probe".into(),kind:SessionKind::Codex,native_id:"readonly-probe".into(),error:None,scope:"test".into(),parent:None};
    let began=std::time::Instant::now();
    store.scan(SessionKind::Codex,&source,&std::sync::atomic::AtomicBool::new(false),true).unwrap();
    let cold=branch.page(None,None,None).unwrap();
    let cold_ms=began.elapsed().as_millis();
    let began=std::time::Instant::now();
    store.scan(SessionKind::Codex,&source,&std::sync::atomic::AtomicBool::new(false),false).unwrap();
    let index_ms=began.elapsed().as_millis();
    let began=std::time::Instant::now();
    let warm=branch.page(None,None,None).unwrap();
    let warm_ms=began.elapsed().as_millis();
    assert!(serde_json::to_vec(&cold).unwrap().len()<=PAGE_BYTES);
    assert!(serde_json::to_vec(&warm).unwrap().len()<=PAGE_BYTES);
    let mut original=std::fs::File::open(&path).unwrap();
    let after=super::source::stamp(&mut original).unwrap();
    assert_eq!(before.identity,after.identity);assert_eq!(before.size,after.size);
    assert_eq!(before.modified,after.modified);assert_eq!(before.head_hash,after.head_hash);
    let conn=store.connect().unwrap();
    let (records,gaps):(u64,u64)=conn.query_row("SELECT count(*),coalesce(sum(error IS NOT NULL),0) FROM records WHERE source_id=?1 AND generation=?2",
        rusqlite::params![source.id,source.generation],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    #[cfg(unix)]
    let rss=unsafe {
        let mut usage=std::mem::MaybeUninit::<libc::rusage>::uninit();
        if libc::getrusage(libc::RUSAGE_SELF,usage.as_mut_ptr())==0 {
            let native=usage.assume_init().ru_maxrss as u64;
            Some(if cfg!(target_os="macos") {native} else {native*1024})
        } else {None}
    };
    #[cfg(not(unix))]
    let rss:Option<u64>=None;
    eprintln!("history_probe {}",serde_json::json!({"sourceBytes":before.size,"coldMs":cold_ms,"indexMs":index_ms,"warmMs":warm_ms,
        "coldRows":cold["rows"].as_array().unwrap().len(),"warmRows":warm["rows"].as_array().unwrap().len(),
        "wireBytes":serde_json::to_vec(&warm).unwrap().len(),"records":records,"gaps":gaps,"coverage":warm["coverage"],
        "indexBytes":std::fs::metadata(&store.path).unwrap().len(),"peakRssBytes":rss,"sourceUnchanged":true}));
}


#[test]
fn duplicate_keys_use_last_content_and_discard_old_checkpoints() {
    let temp=Temp::new();
    let raw=format!("{{\"type\":\"response_item\",\"payload\":{{\"type\":\"function_call_output\",\"call_id\":\"duplicate\",\"output\":\"{}\",\"output\":\"final\"}}}}\n","old".repeat(25000));
    let branch=indexed(&temp,&raw);
    let page=branch.page(None,None,None).unwrap();
    let manifest=branch.manifest(page["rows"][0]["id"].as_str().unwrap(),0).unwrap();
    assert_eq!(manifest["fields"].as_array().unwrap().len(),1);
    let reference=manifest["fields"][0]["detailRef"].as_str().unwrap();
    assert_eq!(branch.detail(reference,None,0).unwrap()["text"],"final");
    assert!(branch.detail(reference,None,1).unwrap_err().starts_with("cursor_expired:"));
}

#[test]
fn long_tool_parameter_named_id_is_content_instead_of_protocol_identity() {
    let raw=serde_json::json!({"type":"assistant","message":{"id":"message","content":[{"type":"tool_use","id":"call","name":"Edit","input":{"id":"x".repeat(40000)}}]}}).to_string()+"\n";
    let mut parser=Projector::new(raw.as_bytes(),0);
    let record=parser.next(&mut |_|Ok(())).unwrap().unwrap();
    assert_eq!(record.error,None);
    assert_eq!(record.value.pointer("/message/content/0/id").unwrap(),"call");
    assert!(record.fields.iter().any(|field|field.path=="/message/content/0/input/id" && field.truncated));
}

#[test]
fn source_native_identity_and_parser_version_invalidate_the_generation() {
    use crate::models::SessionKind;
    let temp=Temp::new();
    let branch=indexed(&temp,"{}\n");
    let old=&branch.sources[0];
    let mut segment=old.segment.clone(); segment.native_id="replacement".into();
    let replaced=branch.store.register(SessionKind::Codex,segment.clone()).unwrap();
    assert_ne!(old.generation,replaced.generation);
    branch.store.connect().unwrap().execute("UPDATE sources SET parser_version=0 WHERE id=?1",[&old.id]).unwrap();
    let migrated=branch.store.register(SessionKind::Codex,segment).unwrap();
    assert_ne!(replaced.generation,migrated.generation);
}

#[test]
fn missing_fork_parent_preserves_verified_child_with_partial_coverage() {
    let temp=Temp::new(); let path=temp.0.join("child.jsonl");
    std::fs::write(&path,"{\"type\":\"session_meta\",\"payload\":{\"id\":\"child\",\"history_base\":{\"thread_id\":\"missing\",\"end_byte_offset\":1}}}\n").unwrap();
    let resolved=super::source::resolve_codex("child",&|id|(id=="child").then(||path.clone())).unwrap();
    assert_eq!(resolved.segments.len(),1);
    assert_eq!(resolved.segments[0].native_id,"child");
    assert!(resolved.error.unwrap().contains("parent rollout file not found"));
}


#[test]
fn cursors_bind_scope_generation_field_and_operation_and_allow_exact_retries() {
    let temp=Temp::new();
    let body="x".repeat(70000);
    let raw=serde_json::json!({"type":"response_item","payload":{"type":"function_call_output","call_id":"cursor","output":body}}).to_string()+"\n";
    let mut branch=indexed(&temp,&raw);
    let row=branch.page(None,None,None).unwrap()["rows"][0]["id"].as_str().unwrap().to_string();
    let manifest=branch.manifest_page(&row,None).unwrap();
    let reference=manifest["fields"][0]["detailRef"].as_str().unwrap();
    let first=branch.detail_page(reference,None,None).unwrap();
    let cursor=first["nextCursor"].as_str().unwrap();
    let second=branch.detail_page(reference,None,Some(cursor)).unwrap();
    assert_eq!(second,branch.detail_page(reference,None,Some(cursor)).unwrap());
    assert_eq!(first,branch.detail_page(reference,None,second["previousCursor"].as_str()).unwrap());
    assert!(branch.manifest_page(&row,Some(cursor)).unwrap_err().starts_with("cursor_expired:"));
    assert!(branch.detail_page("foreign",None,Some(cursor)).unwrap_err().starts_with("cursor_expired:"));
    branch.scope="different-session".into();
    assert!(branch.detail_page(reference,None,Some(cursor)).unwrap_err().starts_with("cursor_expired:"));
}


#[test]
fn unknown_nested_identity_names_are_not_treated_as_protocol_metadata() {
    let raw=serde_json::json!({"unknown":{"type":"x".repeat(40000),"name":"y".repeat(40000)},"type":"response_item","payload":{"type":"message","id":"native","role":"user","content":[{"type":"input_text","text":"visible"}]}}).to_string()+"\n";
    let mut parser=Projector::new(raw.as_bytes(),0);
    let record=parser.next(&mut |_|Ok(())).unwrap().unwrap();
    assert_eq!(record.error,None);assert_eq!(record.value["type"],"response_item");
    assert!(record.fields.iter().filter(|field|field.path.starts_with("/unknown/")).all(|field|field.truncated));
}

#[test]
fn complete_detail_stream_and_http_ranges_preserve_exact_utf8_bytes() {
    let temp=Temp::new();let expected="中文🚀\\\"\n".repeat(18000);
    let raw=serde_json::json!({"type":"response_item","payload":{"type":"function_call_output","call_id":"download","output":expected}}).to_string()+"\n";
    let branch=indexed(&temp,&raw);let page=branch.page(None,None,None).unwrap();
    let manifest=branch.manifest(page["rows"][0]["id"].as_str().unwrap(),0).unwrap();
    let reference=manifest["fields"][0]["detailRef"].as_str().unwrap();let revision=manifest["fields"][0]["contentRevision"].as_str().unwrap();
    let mut bytes=Vec::new();assert_eq!(branch.write_detail(reference,Some(revision),&mut bytes,&std::sync::atomic::AtomicBool::new(false)).unwrap(),expected.len() as u64);
    assert_eq!(bytes,expected.as_bytes());
    assert_eq!(branch.write_detail(reference,Some(revision),&mut Vec::new(),&std::sync::atomic::AtomicBool::new(true)).unwrap_err(),"read_canceled");
    let url=crate::web::download::issue_detail(branch.clone(),reference,Some(revision)).unwrap();
    let runtime=tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    runtime.block_on(async {
        let query=url.split_once('?').unwrap().1.to_owned();
        let mut headers=axum::http::HeaderMap::new();headers.insert(axum::http::header::RANGE,axum::http::HeaderValue::from_static("bytes=10-39999"));
        let response=crate::web::download::handler(axum::extract::RawQuery(Some(query.clone())),headers).await;
        assert_eq!(response.status(),axum::http::StatusCode::PARTIAL_CONTENT);
        let bytes=axum::body::to_bytes(response.into_body(),40000).await.unwrap();assert_eq!(bytes.as_ref(),&expected.as_bytes()[10..40000]);
        let response=crate::web::download::handler(axum::extract::RawQuery(Some(query)),axum::http::HeaderMap::new()).await;
        let bytes=axum::body::to_bytes(response.into_body(),expected.len()).await.unwrap();assert_eq!(bytes.as_ref(),expected.as_bytes());
        let response=crate::web::download::handler(axum::extract::RawQuery(Some("token=invalid".into())),axum::http::HeaderMap::new()).await;
        assert_eq!(response.status(),axum::http::StatusCode::UNAUTHORIZED);
    });
}

#[tokio::test]
async fn markdown_ticket_streams_complete_originals_without_a_staged_export_file() {
        use futures_util::StreamExt;
        let temp=Temp::new();let original="雪😀 original\n".repeat(20_000);
        let raw=serde_json::json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":original}]}}).to_string()+"\n";
        let branch=indexed(&temp,&raw);
        let cancel=std::sync::atomic::AtomicBool::new(false);let mut expected=Vec::new();
        crate::agent::history_store::markdown::write_markdown(branch.clone(),"Complete",None,&cancel,&mut expected).unwrap();
        let url=crate::web::download::issue_markdown(branch,"Complete",None).unwrap();
        let response=crate::web::download::handler(axum::extract::RawQuery(url.split_once('?').map(|(_,query)|query.to_owned())),axum::http::HeaderMap::new()).await;
        assert_eq!(response.status(),axum::http::StatusCode::OK);assert_eq!(response.headers()[axum::http::header::ACCEPT_RANGES],"none");
        let mut stream=response.into_body().into_data_stream();let mut actual=Vec::new();
        while let Some(bytes)=stream.next().await {let bytes=bytes.unwrap();assert!(bytes.len()<=32*1024);actual.extend_from_slice(&bytes);}
        assert_eq!(actual,expected);assert!(!temp.0.join("chat-exports").exists());
    }

#[test]
fn a_running_process_pages_only_rows_recorded_before_its_launch() {
    use crate::models::SessionKind;
    fn turn(raw:&mut String,parent:&mut Option<String>,index:usize) {
        for (kind,text) in [("user",format!("Question {index}")),("assistant",format!("Answer {index}"))] {
            let uuid=format!("00000000-0000-4000-8000-{index:06}{:06}",if kind=="user" {0} else {1});
            let message=if kind=="user" {serde_json::json!({"role":"user","content":[{"type":"text","text":text}]})}
                else {serde_json::json!({"id":format!("msg_{index}"),"type":"message","role":"assistant","content":[{"type":"text","text":text}]})};
            raw.push_str(&serde_json::json!({"type":kind,"uuid":uuid,"parentUuid":parent,"sessionId":"thread","timestamp":"2026-10-01T00:00:00Z","message":message}).to_string());
            raw.push('\n');*parent=Some(uuid);
        }
    }
    let temp=Temp::new();let path=temp.0.join("native.jsonl");
    let (mut raw,mut parent)=(String::new(),None);for index in 0..3 {turn(&mut raw,&mut parent,index);}
    let branch=indexed_kind(&temp,&raw,SessionKind::Claude);
    let cut=super::api::ProcessCut {epoch:7,sizes:vec![(path.clone(),raw.len() as u64)]};
    assert!(cut.has_history());
    // The launched process appends a turn; the index has not caught up with it yet.
    let mut appended=String::new();turn(&mut appended,&mut parent,3);
    std::fs::OpenOptions::new().append(true).open(&path).unwrap().write_all(appended.as_bytes()).unwrap();
    let source=branch.store.register(SessionKind::Claude,super::source::Segment {path:path.clone(),native_id:"thread".into(),end_byte:None,end_ordinal:None}).unwrap();
    let full=super::api::Branch {sources:vec![source.clone()],..branch.clone()};
    let capped=full.before_process(&cut).unwrap();
    let page=capped.page_with(None,None,None,true).unwrap();
    let texts=page["rows"].as_array().unwrap().iter().map(|row|row["text"].as_str().unwrap().to_owned()).collect::<Vec<_>>();
    assert_eq!(texts,["Question 0","Answer 0","Question 1","Answer 1","Question 2","Answer 2"]);
    assert_eq!(page["indexing"],false);assert_eq!(page["knownRows"],6);
    // An ordinary page of a tree provider shows the branch its last complete scan confirmed until
    // the leaf at the end of the file has been read.
    let pending=full.page(None,None,None).unwrap();
    assert_eq!(pending["rows"].as_array().unwrap().len(),6);assert_eq!(pending["coverage"],"partial");
    branch.store.scan(SessionKind::Claude,&source,&std::sync::atomic::AtomicBool::new(false),false).unwrap();
    assert_eq!(full.page(None,None,None).unwrap()["knownRows"],8);
    let page=capped.page_with(None,None,None,true).unwrap();assert_eq!(page["knownRows"],6);
    let first=page["rows"][2]["id"].as_str().unwrap();
    let older=capped.page_with(Some(first),None,None,true).unwrap();
    assert_eq!(older["rows"].as_array().unwrap().len(),2);assert_eq!(older["hasMore"],false);
    // A recording created by the process itself has no earlier rows.
    let none=super::api::ProcessCut {epoch:7,sizes:Vec::new()};
    assert!(!none.has_history());
    assert!(full.before_process(&none).unwrap().page_with(None,None,None,true).unwrap()["rows"].as_array().unwrap().is_empty());
}

#[test]
fn codex_recordings_with_completed_items_show_each_message_and_tool_once() {
    use crate::models::SessionKind;
    use serde_json::json;
    let response=|payload:serde_json::Value|json!({"timestamp":"2026-10-14T07:21:03Z","type":"response_item","payload":payload});
    let item=|item:serde_json::Value|json!({"timestamp":"2026-10-14T07:21:04Z","type":"event_msg","payload":{"type":"item_completed","item":item}});
    let lines=[
        response(json!({"type":"message","id":"msg_user","role":"user","content":[{"type":"input_text","text":"Create codex.txt"}]})),
        item(json!({"type":"UserMessage","id":"user-item","client_id":"client-1","content":[{"type":"text","text":"Create codex.txt"}]})),
        item(json!({"type":"AgentMessage","id":"msg_answer","content":[{"type":"Text","text":"Working on it"}]})),
        response(json!({"type":"message","id":"msg_answer","role":"assistant","content":[{"type":"output_text","text":"Working on it"}]})),
        response(json!({"type":"custom_tool_call","id":"ctc_1","call_id":"call_file","name":"exec","input":"write codex.txt"})),
        item(json!({"type":"FileChange","id":"exec-file","status":"completed","changes":{"/work/codex.txt":{"type":"add","content":"red\ngreen\nblue\n"}}})),
        response(json!({"type":"custom_tool_call_output","id":"ctco_1","call_id":"call_file","output":"ok"})),
        response(json!({"type":"custom_tool_call","id":"ctc_2","call_id":"call_cmd","name":"exec","input":"seq 1 3000 | tail -n 1"})),
        item(json!({"type":"CommandExecution","id":"exec-cmd","command":["seq"],"cwd":"/work","status":"completed","aggregated_output":"3000\n","exit_code":0})),
        response(json!({"type":"custom_tool_call_output","id":"ctco_2","call_id":"call_cmd","output":"3000"})),
        response(json!({"type":"message","id":"msg_user_2","role":"user","content":[{"type":"input_text","text":"Again"}]})),
        item(json!({"type":"UserMessage","id":"user-item-2","client_id":"client-2","content":[{"type":"text","text":"Again"}]})),
    ];
    let raw=lines.iter().map(|line|format!("{line}\n")).collect::<String>();
    let temp=Temp::new();let branch=indexed_kind(&temp,&raw,SessionKind::Codex);
    let view=|branch:&super::api::Branch| branch.page(None,None,None).unwrap()["rows"].as_array().unwrap().iter()
        .map(|row|format!("{}:{}",row["kind"].as_str().unwrap(),row["name"].as_str().or(row["text"].as_str()).unwrap_or_default())).collect::<Vec<_>>();
    let expected=["user:Create codex.txt","assistant:Working on it","tool:FileChange","tool:Bash","user:Again"];
    assert_eq!(view(&branch),expected);
    // Transcript prose still reads the response items, so every message stays authorized and readable.
    let cancel=std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let prose=std::rc::Rc::new(super::prose::Prose::new(branch.clone(),&cancel).unwrap());assert!(prose.total>=3);
    for number in 0..prose.total {
        let mut text=String::new();super::prose_reader::Reader::new(prose.clone(),number,cancel.clone()).unwrap().read_to_string(&mut text).unwrap();
        assert!(!text.is_empty());
        // A message located through its response-item row lands on the row shown in the view.
        if let Some(row)=prose.row_id(number).unwrap() {
            let page=branch.page(None,None,Some(&row)).unwrap();let located=page["locatedRowId"].as_str().unwrap();
            assert!(page["rows"].as_array().unwrap().iter().any(|shown|shown["id"]==located));
        }
    }
    // A scan started from an earlier registration replays the same records to the same rows.
    let stale=super::store::RegisteredSource {indexed:0,..branch.sources[0].clone()};
    branch.store.scan(SessionKind::Codex,&stale,&std::sync::atomic::AtomicBool::new(false),false).unwrap();
    assert_eq!(view(&branch),expected);
    // A recording without completed items keeps its response items.
    let older=lines.iter().filter(|line|line["type"]=="response_item").map(|line|format!("{line}\n")).collect::<String>();
    let temp=Temp::new();let branch=indexed_kind(&temp,&older,SessionKind::Codex);
    let rows=view(&branch);
    assert_eq!(rows.iter().filter(|row|row.starts_with("user:")).count(),2);assert_eq!(rows.iter().filter(|row|row.starts_with("tool:")).count(),2);
}

#[test]
fn reclaiming_free_pages_runs_the_vacuum_to_completion() {
    let temp=Temp::new();let conn=rusqlite::Connection::open(temp.0.join("vacuum.sqlite")).unwrap();
    conn.execute_batch("PRAGMA auto_vacuum=INCREMENTAL; VACUUM; CREATE TABLE t(v BLOB);").unwrap();
    for _ in 0..2000 {conn.execute("INSERT INTO t VALUES(zeroblob(4000))",[]).unwrap();}
    conn.execute("DELETE FROM t",[]).unwrap();
    let free=|conn:&rusqlite::Connection| conn.query_row("PRAGMA freelist_count",[],|row|row.get::<_,u64>(0)).unwrap();
    let before=free(&conn);assert!(before>1000);
    // A single step frees a single page; this is why every earlier call left the file growing.
    conn.execute_batch("PRAGMA incremental_vacuum;").unwrap();assert!(free(&conn)>=before-1);
    super::store::reclaim_free_pages(&conn).unwrap();assert_eq!(free(&conn),0);
}

#[test]
fn a_checkpointed_wal_is_not_mistaken_for_retained_reads() {
    let temp=Temp::new();let store=super::store::Store::at(&temp.0).unwrap();let conn=store.connect().unwrap();
    conn.execute_batch("CREATE TABLE filler(v BLOB); BEGIN;").unwrap();
    for _ in 0..12000 {conn.execute("INSERT INTO filler VALUES(zeroblob(4000))",[]).unwrap();}
    conn.execute_batch("COMMIT; DELETE FROM filler;").unwrap();
    let wal=temp.0.join("chat-index/index.sqlite-wal");
    // Every frame has been written back; only the file's earlier size remains.
    conn.query_row("PRAGMA wal_checkpoint(PASSIVE)",[],|_|Ok(())).unwrap();
    super::cache::check_wal(&store).unwrap();
    assert!(std::fs::metadata(&wal).map(|meta|meta.len()).unwrap_or(0)<=super::cache::WAL_RESERVE);
    // A reader that still needs old frames keeps the guard.
    let reader=store.connect().unwrap();reader.execute_batch("BEGIN; SELECT count(*) FROM filler;").unwrap();
    conn.execute_batch("BEGIN;").unwrap();for _ in 0..12000 {conn.execute("INSERT INTO filler VALUES(zeroblob(4000))",[]).unwrap();}conn.execute_batch("COMMIT;").unwrap();
    assert!(super::cache::check_wal(&store).unwrap_err().starts_with("storage_pressure:"));
    reader.execute_batch("COMMIT;").unwrap();super::cache::check_wal(&store).unwrap();
    // Another scan writing at the same time blocks the truncation but retains nothing.
    conn.execute_batch("BEGIN;").unwrap();for _ in 0..12000 {conn.execute("INSERT INTO filler VALUES(zeroblob(4000))",[]).unwrap();}conn.execute_batch("COMMIT;").unwrap();
    let writer=store.connect().unwrap();writer.execute_batch("BEGIN IMMEDIATE; INSERT INTO filler VALUES(1);").unwrap();
    assert!(std::fs::metadata(&wal).map(|meta|meta.len()).unwrap_or(0)>super::cache::WAL_RESERVE);
    super::cache::check_wal(&store).unwrap();
    writer.execute_batch("COMMIT;").unwrap();
    drop(writer);drop(reader);drop(conn);
}



#[test]
fn the_derived_budget_grows_to_fit_one_large_recording() {
    let temp=Temp::new();let store=super::store::Store::at(&temp.0).unwrap();let initial=store.budget();
    let path=temp.0.join("large.jsonl");let file=std::fs::File::create(&path).unwrap();file.set_len(initial).unwrap();
    store.register(crate::models::SessionKind::Codex,super::source::Segment {path,native_id:"large".into(),end_byte:None,end_ordinal:None}).unwrap();
    assert!(store.budget()>=initial*2,"a recording as large as the budget fits after registration");
    let small=temp.0.join("small.jsonl");std::fs::write(&small,"").unwrap();let grown=store.budget();
    store.register(crate::models::SessionKind::Codex,super::source::Segment {path:small,native_id:"small".into(),end_byte:None,end_ordinal:None}).unwrap();
    assert_eq!(store.budget(),grown,"a smaller recording never shrinks the budget");
}

#[test]
fn a_scalar_identity_field_verifies_only_its_own_bytes() {
    use std::io::{Seek,SeekFrom,Write};
    let temp=Temp::new();
    let raw=serde_json::json!({"type":"user","uuid":"one","isSidechain":false,"message":{"role":"user","content":"Hello"}}).to_string()+"\n";
    let branch=indexed_kind(&temp,&raw,crate::models::SessionKind::Claude);let source=&branch.sources[0];let conn=branch.store.connect().unwrap();
    let (reference,hash):(String,Option<Vec<u8>>)=conn.query_row("SELECT ref,hash FROM fields WHERE source_id=?1 AND path='/isSidechain'",[&source.id],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert!(hash.is_some(),"a scalar identity carries the hash of its literal");
    assert_eq!(super::api::read_piece(&conn,source,&reference,0,0).unwrap().0,"false");
    // An edit in place keeps the file identity; the field's own hash still notices it.
    let at=raw.find("false").unwrap() as u64;let mut file=std::fs::OpenOptions::new().write(true).open(&source.segment.path).unwrap();
    file.seek(SeekFrom::Start(at)).unwrap();file.write_all(b"fakse").unwrap();drop(file);
    assert!(super::api::read_piece(&conn,source,&reference,0,0).unwrap_err().starts_with("source_changed:"));
}

#[test]
fn a_compaction_repeating_a_long_history_does_not_make_the_recording_unreadable() {
    let temp=Temp::new();
    let history=(0..5000).map(|index|serde_json::json!({"type":"message","role":"user","content":[{"type":"input_text","text":format!("old {index}")}]})).collect::<Vec<_>>();
    let compacted=serde_json::json!({"type":"compacted","payload":{"message":"","replacement_history":history,"replacement_history_metadata":[{"kind":"x"}]}});
    let after=serde_json::json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"After compaction"}]}});
    let branch=indexed(&temp,&format!("{compacted}\n{after}\n"));let conn=branch.store.connect().unwrap();
    let (status,errors):(String,u64)=conn.query_row("SELECT status,(SELECT count(*) FROM records WHERE error IS NOT NULL) FROM sources",[],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!((status.as_str(),errors),("complete",0));
    let rows=branch.page(None,None,None).unwrap()["rows"].as_array().unwrap().clone();
    assert!(rows.iter().any(|row|row.to_string().contains("After compaction")));
}

#[test]
fn a_tree_recording_being_appended_keeps_showing_its_confirmed_branch() {
    let temp=Temp::new();
    let line=|uuid:&str,parent:Option<&str>,text:&str| serde_json::json!({"type":"user","uuid":uuid,"parentUuid":parent,"message":{"role":"user","content":text}}).to_string()+"\n";
    let raw=line("a",None,"First question")+&line("b",Some("a"),"Second question");
    let branch=indexed_kind(&temp,&raw,crate::models::SessionKind::Claude);
    assert_eq!(branch.page(None,None,None).unwrap()["rows"].as_array().unwrap().len(),2);
    // Another process appends; the tail is registered but not read yet.
    let path=branch.sources[0].segment.path.clone();
    std::fs::OpenOptions::new().append(true).open(&path).unwrap().write_all(line("c",Some("b"),"Third question").as_bytes()).unwrap();
    let source=branch.store.register(crate::models::SessionKind::Claude,super::source::Segment {path,native_id:"thread".into(),end_byte:None,end_ordinal:None}).unwrap();
    assert_eq!(source.status,"partial");
    let appended=super::api::Branch {sources:vec![source],..branch.clone()};
    let page=appended.page(None,None,None).unwrap();
    assert_eq!(page["coverage"],"partial");
    assert_eq!(page["rows"].as_array().unwrap().len(),2,"the branch confirmed before the append stays visible: {page}");
}

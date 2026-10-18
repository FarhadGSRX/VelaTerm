//! Original image fields remain source references. Preview and download reads use verified chunks.

use std::collections::HashMap;
use std::io::{Read,Write};
use std::sync::atomic::{AtomicBool,Ordering};
use base64::{Engine,engine::general_purpose::STANDARD};
use rusqlite::{params,Connection,OptionalExtension};
use serde_json::{json,Value};
use super::{api::{Branch,DecodedFieldReader},projection::Record,store::{RegisteredSource,sql}};

fn data_url(text:&str) -> Option<(&str,usize)> {
    let (mime,rest)=text.strip_prefix("data:")?.split_once(';')?;
    if mime.len()>100 || !mime.starts_with("image/") || !rest.starts_with("base64,") {return None;}
    Some((mime,5+mime.len()+8))
}

pub(super) fn attach(conn:&Connection,source:&RegisteredSource,record:&Record,row:&str) -> Result<(),String> {
    let mut selected=Vec::new();
    let mut statement=conn.prepare("SELECT path,decoded_bytes FROM fields WHERE source_id=?1 AND generation=?2 AND record_start=?3 AND encoding='string' ORDER BY raw_start LIMIT 4097").map_err(sql)?;
    let fields=statement.query_map(params![source.id,source.generation,record.start],|row|Ok((row.get::<_,String>(0)?,row.get::<_,u64>(1)?))).map_err(sql)?;
    for (index,field) in fields.enumerate() {
        if index>=super::limits::MAX_FIELDS {return Err("unsupported_record: image field directory exceeds its entry budget".into());}
        let (path,size)=field.map_err(sql)?;let path=path.as_str();let value=&record.value;
        let image=if path.starts_with("/images/") && path.ends_with("/data") && value["type"]=="dispatch" {
            value.pointer(path.strip_suffix("/data").unwrap()).and_then(|image|image["mimeType"].as_str()).map(|mime|(mime,0))
        } else if let Some(root)=path.strip_suffix("/source/data").filter(|root|root.starts_with("/message/content/")) {
            value.pointer(root).filter(|image|image["type"]=="image" && image.pointer("/source/type").and_then(Value::as_str)==Some("base64")).and_then(|image|image.pointer("/source/media_type").and_then(Value::as_str)).map(|mime|(mime,0))
        } else if let Some(root)=path.strip_suffix("/data").filter(|root|root.starts_with("/message/content/")) {
            value.pointer(root).filter(|image|image["type"]=="image").and_then(|image|image["mimeType"].as_str()).map(|mime|(mime,0))
        } else if (path.ends_with("/image_url") || path.ends_with("/imageUrl"))
                && (path.starts_with("/payload/content/") || path.starts_with("/payload/item/content/") || path.starts_with("/params/item/content/"))
            || path=="/part/url" && value.pointer("/part/type").and_then(Value::as_str)==Some("file") {
            value.pointer(path).and_then(Value::as_str).and_then(data_url)
        } else {None};
        let Some((mime,prefix))=image.filter(|(mime,_)|mime.len()<=100 && mime.starts_with("image/") && mime.bytes().all(|byte|byte.is_ascii_alphanumeric() || matches!(byte,b'/'|b'-'|b'.'|b'+'))) else {continue;};
        if size<prefix as u64 {return Err("unsupported_record: image original has an invalid data URL boundary".into());}
        selected.push((path.to_owned(),format!("attachment:{mime}:{prefix}")));
    }
    for (path,label) in selected {
        let reference:String=conn.query_row("SELECT ref FROM fields WHERE source_id=?1 AND generation=?2 AND record_start=?3 AND path=?4",
            params![source.id,source.generation,record.start,path],|row|row.get(0)).map_err(sql)?;
        super::adapter::attach(conn,source,row,&label,&[path.clone()],&HashMap::from([(path,reference)]))?;
    }Ok(())
}

pub(super) fn decorate(conn:&Connection,source:&RegisteredSource,row:Value) -> Result<Value,String> {
    if row["kind"]!="user" {return Ok(row);}
    let mut statement=conn.prepare("SELECT f.ref,rf.label,f.decoded_bytes FROM row_fields rf JOIN fields f ON f.ref=rf.ref
        JOIN records r ON r.source_id=f.source_id AND r.generation=f.generation AND r.start=f.record_start
        WHERE rf.source_id=?1 AND rf.generation=?2 AND rf.row_id=?3 AND rf.label LIKE 'attachment:%' AND r.active=1
        AND (?4 IS NULL OR r.end<=?4) AND (?5 IS NULL OR r.ordinal IS NULL OR r.ordinal<?5) ORDER BY rf.sequence LIMIT 20").map_err(sql)?;
    let entries=statement.query_map(params![source.id,source.generation,row["id"].as_str(),source.segment.end_byte,source.segment.end_ordinal],
        |row|Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?,row.get::<_,u64>(2)?))).map_err(sql)?;
    let mut images=Vec::new();for entry in entries {let (reference,label,bytes)=entry.map_err(sql)?;
        let (mime,prefix)=label.strip_prefix("attachment:").and_then(|label|label.rsplit_once(':')).ok_or("index_incomplete: image metadata is invalid")?;
        let prefix:u64=prefix.parse().map_err(|_|"index_incomplete: image boundary is invalid")?;
        images.push(json!({"attachmentId":format!("history:{reference}"),"mimeType":mime,"sizeBytes":bytes.saturating_sub(prefix),"contentRevision":format!("{}:{reference}",source.generation)}));
    }
    let mut row=row;if !images.is_empty() {row["images"]=json!(images);row["bodyAvailable"]=json!(true);}Ok(row)
}

pub struct Facts {pub mime:String,pub prefix:u64,pub encoded:u64,pub revision:String,pub source:RegisteredSource}
pub fn is_attachment(branch:&Branch,reference:&str) -> Result<bool,String> {
    branch.authorize(reference)?;
    branch.store.connect()?.query_row("SELECT EXISTS(SELECT 1 FROM row_fields WHERE ref=?1 AND label LIKE 'attachment:%')",[reference],|row|row.get(0)).map_err(sql)
}
pub fn facts(branch:&Branch,reference:&str,expected:Option<&str>) -> Result<Facts,String> {
    let (source,start,path,size,_)=branch.authorize(reference)?;let (revision,_)=branch.detail_facts(reference,expected)?;
    let label:Option<String>=branch.store.connect()?.query_row("SELECT label FROM row_fields WHERE source_id=?1 AND generation=?2 AND ref=?3 AND label LIKE 'attachment:%' LIMIT 1",
        params![source.id,source.generation,reference],|row|row.get(0)).optional().map_err(sql)?;
    let label=label.ok_or("detail_unavailable: this detail is not an image attachment")?;
    let (mime,prefix)=label.strip_prefix("attachment:").and_then(|label|label.rsplit_once(':')).ok_or("index_incomplete: image metadata is invalid")?;
    let prefix=prefix.parse::<u64>().map_err(|_|"index_incomplete: image boundary is invalid")?;
    let encoded=size.checked_sub(prefix).ok_or("index_incomplete: image boundary is outside the original")?;
    let conn=branch.store.connect()?;
    if prefix>0 {
        let (head,_,_,_)=super::api::read_piece(&conn,&source,reference,0,0)?;
        if !head.starts_with(&format!("data:{mime};base64,")) {return Err("source_changed: image data URL identity changed".into());}
    } else {
        let metadata=if let Some(root)=path.strip_suffix("/source/data") {format!("{root}/source/media_type")}
            else if let Some(root)=path.strip_suffix("/data") {format!("{root}/mimeType")}
            else {return Err("index_incomplete: image metadata has no verified original field".into());};
        let reference:String=conn.query_row("SELECT ref FROM fields WHERE source_id=?1 AND generation=?2 AND record_start=?3 AND path=?4",
            params![source.id,source.generation,start,metadata],|row|row.get(0)).map_err(sql)?;
        let (value,_,_,length)=super::api::read_piece(&conn,&source,&reference,0,0)?;
        if length>100 || value!=mime {return Err("source_changed: image media type changed".into());}
    }
    Ok(Facts {mime:mime.into(),prefix,encoded,revision,source})
}

pub fn write(branch:&Branch,reference:&str,expected:Option<&str>,writer:&mut impl Write,cancel:&AtomicBool) -> Result<u64,String> {
    let facts=facts(branch,reference,expected)?;let conn=branch.store.connect()?;
    let reader=DecodedFieldReader::new(&conn,&facts.source,reference,facts.prefix,None,0)?;
    let mut reader=base64::read::DecoderReader::new(reader,&STANDARD);let mut buffer=[0;super::limits::CHECKPOINT_BYTES];let mut bytes=0u64;
    loop {
        if cancel.load(Ordering::Relaxed) {return Err("read_canceled".into());}
        // Recheck the parent scope and attachment revision before every separately verified chunk.
        branch.detail_facts(reference,Some(&facts.revision))?;
        let count=reader.read(&mut buffer).map_err(|error|format!("detail_unavailable: image original cannot be decoded: {error}"))?;
        if count==0 {break;}writer.write_all(&buffer[..count]).map_err(|_|"storage_pressure: image original could not be written")?;bytes+=count as u64;
    }self::facts(branch,reference,Some(&facts.revision))?;Ok(bytes)
}

// Header inspection is bounded and never allocates pixel buffers. Formats without a verified size
// remain downloadable; a preview must not cause an unbounded browser image decode.
fn dimensions(bytes:&[u8]) -> Option<(u32,u32)> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") && bytes.len()>=24 {
        return Some((u32::from_be_bytes(bytes[16..20].try_into().ok()?),u32::from_be_bytes(bytes[20..24].try_into().ok()?)));
    }
    if (bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a")) && bytes.len()>=10 {
        return Some((u16::from_le_bytes(bytes[6..8].try_into().ok()?) as u32,u16::from_le_bytes(bytes[8..10].try_into().ok()?) as u32));
    }
    if bytes.starts_with(b"\xff\xd8") {
        let mut at=2;while at+4<=bytes.len() {
            if bytes[at]!=0xff {return None;}while at<bytes.len() && bytes[at]==0xff {at+=1;}
            let marker=*bytes.get(at)?;at+=1;if matches!(marker,0xd9|0xda) {return None;}
            if marker==0x01 || (0xd0..=0xd7).contains(&marker) {continue;}
            let size=u16::from_be_bytes(bytes.get(at..at+2)?.try_into().ok()?) as usize;
            if size<2 || at+size>bytes.len() {return None;}
            if matches!(marker,0xc0..=0xc3|0xc5..=0xc7|0xc9..=0xcb|0xcd..=0xcf) && size>=7 {
                return Some((u16::from_be_bytes(bytes[at+5..at+7].try_into().ok()?) as u32,u16::from_be_bytes(bytes[at+3..at+5].try_into().ok()?) as u32));
            }at+=size;
        }
    }
    if bytes.len()>=30 && &bytes[..4]==b"RIFF" && &bytes[8..12]==b"WEBP" {
        if &bytes[12..16]==b"VP8X" {return Some((1+u32::from_le_bytes([bytes[24],bytes[25],bytes[26],0]),1+u32::from_le_bytes([bytes[27],bytes[28],bytes[29],0])));}
        if &bytes[12..16]==b"VP8 " && &bytes[23..26]==b"\x9d\x01\x2a" {return Some(((u16::from_le_bytes(bytes[26..28].try_into().ok()?)&0x3fff) as u32,(u16::from_le_bytes(bytes[28..30].try_into().ok()?)&0x3fff) as u32));}
        if &bytes[12..16]==b"VP8L" && bytes[20]==0x2f {
            let bits=u32::from_le_bytes(bytes[21..25].try_into().ok()?);return Some((1+(bits&0x3fff),1+((bits>>14)&0x3fff)));
        }
    }None
}

fn preview_guard() -> Result<std::sync::MutexGuard<'static,()>,String> {
    static PREVIEWS:std::sync::OnceLock<std::sync::Mutex<()>>=std::sync::OnceLock::new();
    // One explicit image read at a time bounds decoded pixels, original bytes, and encoding copies.
    PREVIEWS.get_or_init(||std::sync::Mutex::new(())).lock().map_err(|_|"index_busy: image preview reader is unavailable".into())
}
fn image(bytes:Vec<u8>,mime:String,original:bool,view:bool) -> Result<crate::agent::chat::protocol::ChatImage,String> {
    if bytes.len()>crate::command_core::MAX_IMAGE_BYTES {return Err("image_requires_download: image original exceeds the attachment read budget".into());}
    if !original || view {
        let (width,height)=dimensions(&bytes[..bytes.len().min(128*1024)]).ok_or("image_requires_download: image dimensions cannot be verified for a preview")?;
        if width==0 || height==0 || width>16384 || height>16384 || width as u64*height as u64>4*1024*1024 {
            return Err("image_requires_download: image dimensions exceed the preview budget".into());
        }
    }
    if !original {
        let mut reader=image::ImageReader::new(std::io::Cursor::new(&bytes)).with_guessed_format().map_err(|_|"image_requires_download: image format cannot be previewed")?;
        let mut limits=image::Limits::default();limits.max_image_width=Some(16384);limits.max_image_height=Some(16384);limits.max_alloc=Some(32*1024*1024);reader.limits(limits);
        let image=reader.decode().map_err(|_|"image_requires_download: image cannot be decoded within the preview budget")?;
        let thumbnail=image.thumbnail(256,256);drop(image);drop(bytes);let mut output=std::io::Cursor::new(Vec::new());
        thumbnail.write_to(&mut output,image::ImageFormat::Png).map_err(|_|"detail_unavailable: image preview could not be encoded")?;
        let bytes=output.into_inner();if bytes.len()>384*1024 {return Err("image_requires_download: image preview exceeds its byte budget".into());}
        return Ok(crate::agent::chat::protocol::ChatImage {mime_type:"image/png".into(),data:STANDARD.encode(bytes)});
    }
    Ok(crate::agent::chat::protocol::ChatImage {mime_type:mime,data:STANDARD.encode(bytes)})
}

pub fn resolve_owned(original:crate::agent::chat::protocol::ChatImage,purpose:&str) -> Result<crate::agent::chat::protocol::ChatImage,String> {
    let _preview=preview_guard()?;resolve_owned_locked(original,purpose)
}
pub fn resolve_runtime(purpose:&str,read:impl FnOnce()->Result<crate::agent::chat::protocol::ChatImage,String>) -> Result<crate::agent::chat::protocol::ChatImage,String> {
    // Acquire the codec budget before collecting a live original, so simultaneous mounted previews
    // cannot each retain a full encoded image while waiting for their turn to decode.
    let _preview=preview_guard()?;resolve_owned_locked(read()?,purpose)
}
fn resolve_owned_locked(original:crate::agent::chat::protocol::ChatImage,purpose:&str) -> Result<crate::agent::chat::protocol::ChatImage,String> {
    if original.data.len() as u64>4*((crate::command_core::MAX_IMAGE_BYTES as u64+2)/3) {return Err("image_requires_download: image original exceeds the attachment read budget".into());}
    if purpose=="original" {return Ok(original);}
    let bytes=STANDARD.decode(&original.data).map_err(|_|"detail_unavailable: image original has invalid Base64 encoding")?;
    image(bytes,original.mime_type,purpose=="view",purpose=="view")
}

pub fn resolve(branch:&Branch,reference:&str,expected:Option<&str>,original:bool) -> Result<crate::agent::chat::protocol::ChatImage,String> {
    let _preview=preview_guard()?;
    let facts=facts(branch,reference,expected)?;
    if facts.encoded>4*((crate::command_core::MAX_IMAGE_BYTES as u64+2)/3) {return Err("image_requires_download: image original exceeds the attachment read budget".into());}
    let cancel=AtomicBool::new(false);let mut bytes=Vec::with_capacity((facts.encoded/4*3) as usize);
    write(branch,reference,Some(&facts.revision),&mut bytes,&cancel)?;
    image(bytes,facts.mime,original,false)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn image() -> Vec<u8> {
        let mut seed=719u32;let image=image::RgbImage::from_fn(600,400,|_,_| {
            let mut pixel=[0;3];for byte in &mut pixel {seed^=seed<<13;seed^=seed>>17;seed^=seed<<5;*byte=seed as u8;}image::Rgb(pixel)
        });let mut bytes=std::io::Cursor::new(Vec::new());image.write_to(&mut bytes,image::ImageFormat::Png).unwrap();bytes.into_inner()
    }

    fn native(kind:crate::models::SessionKind,encoded:&str) -> Value {
        use crate::models::SessionKind;
        match kind {
            SessionKind::Codex=>json!({"type":"response_item","payload":{"type":"message","id":"user","role":"user","content":[{"type":"input_text","text":"Question"},{"type":"input_image","image_url":format!("data:image/png;base64,{encoded}")}]}}),
            SessionKind::Claude=>json!({"type":"user","uuid":"user","message":{"role":"user","content":[{"type":"text","text":"Question"},{"type":"image","source":{"type":"base64","media_type":"image/png","data":encoded}}]}}),
            _=>json!({"type":"message","id":"user","message":{"role":"user","content":[{"type":"text","text":"Question"},{"type":"image","mimeType":"image/png","data":encoded}]}}),
        }
    }
    fn reference(branch:&Branch) -> String {
        let page=branch.page(None,None,None).unwrap();assert!(page.to_string().len()<16*1024);
        let row=&page["rows"][0];assert_eq!(row["images"].as_array().unwrap().len(),1);assert_eq!(row["bodyAvailable"],true);
        let reference=row["images"][0]["attachmentId"].as_str().unwrap().strip_prefix("history:").unwrap().to_owned();
        let manifest=branch.manifest(row["id"].as_str().unwrap(),0).unwrap();
        assert!(manifest["fields"].as_array().unwrap().iter().any(|field|field["detailRef"]==reference && field["kind"]=="image" && field["mimeType"]=="image/png"));reference
    }
    #[test]
    fn native_images_remain_small_references_and_restore_exact_originals_or_bounded_thumbnails() {
        use crate::models::SessionKind;
        let bytes=image();let encoded=STANDARD.encode(&bytes);assert!(encoded.len()>super::super::limits::PAGE_BYTES);
        for kind in [SessionKind::Codex,SessionKind::Claude,SessionKind::Pi,SessionKind::Omp] {
            let temp=super::super::tests::Temp::new();let raw=native(kind,&encoded).to_string()+"\n";
            let branch=super::super::tests::indexed_kind(&temp,&raw,kind);let reference=reference(&branch);
            let original=resolve(&branch,&reference,None,true).unwrap();assert_eq!(original.data,encoded);assert_eq!(original.mime_type,"image/png");
            let preview=resolve(&branch,&reference,None,false).unwrap();assert!(preview.data.len()<super::super::limits::PAGE_BYTES);
            let thumbnail=image::load_from_memory(&STANDARD.decode(&preview.data).unwrap()).unwrap();assert_eq!((thumbnail.width(),thumbnail.height()),(256,171));
            let mut restored=Vec::new();assert_eq!(write(&branch,&reference,None,&mut restored,&AtomicBool::new(false)).unwrap(),bytes.len() as u64);assert_eq!(restored,bytes);
            let other=super::super::tests::Temp::new();let foreign=super::super::tests::indexed_kind(&other,&(native(kind,"YQ==").to_string()+"\n"),kind);
            assert!(facts(&foreign,&reference,None).is_err());assert!(resolve(&branch,&reference,Some("old-generation"),true).unwrap_err().starts_with("source_changed:"));
            assert_eq!(std::fs::read_to_string(&branch.sources[0].segment.path).unwrap(),raw);
            std::fs::write(&branch.sources[0].segment.path,raw.replacen(&encoded[..100],&"A".repeat(100),1)).unwrap();
            assert!(resolve(&branch,&reference,None,true).unwrap_err().contains("source_changed:"));
        }
    }

    #[test]
    fn outgoing_images_are_hidden_until_delivery_and_survive_index_rebuild() {
        let temp=super::super::tests::Temp::new();let archive=super::super::live::Archive::at(&temp.0).unwrap();let cancel=AtomicBool::new(false);
        let encoded=STANDARD.encode(image());let mut journal=super::super::live::Journal::new(archive.clone(),"scope",7).unwrap();
        journal.append("dispatch:image",&json!({"type":"dispatch","id":"image","text":"Question","images":[{"mimeType":"image/png","data":encoded}]}),&cancel).unwrap();
        let frame=journal.append("delivery:image",&json!({"type":"delivery","id":"image","state":"pipe_written"}),&cancel).unwrap();
        archive.bind_outgoing(&frame,crate::models::SessionKind::Codex,"scope",7,"thread").unwrap();
        let store=super::super::store::Store::at(&temp.0).unwrap();let source=super::super::protocol_source::register(&store,crate::models::SessionKind::Codex,&frame.reference,"scope",7,"thread").unwrap();
        store.scan(crate::models::SessionKind::Codex,&source,&cancel,false).unwrap();
        let branch=Branch {store,sources:vec![source],generation:"branch".into(),kind:crate::models::SessionKind::Codex,native_id:"thread".into(),error:None,scope:"scope".into(),parent:None};
        let reference=reference(&branch);assert_eq!(resolve(&branch,&reference,None,true).unwrap().data,encoded);
        let conn=branch.store.connect().unwrap();conn.execute_batch("DELETE FROM rows;DELETE FROM row_parts;DELETE FROM row_fields;UPDATE sources SET indexed_bytes=0,status='partial'").unwrap();drop(conn);
        branch.store.scan(branch.kind,&branch.sources[0],&cancel,false).unwrap();assert_eq!(self::reference(&branch),reference);assert_eq!(resolve(&branch,&reference,None,true).unwrap().data,encoded);
    }

    #[test]
    fn downloads_stream_large_originals_and_cancellation_stops_after_one_bounded_chunk() {
        let temp=super::super::tests::Temp::new();let mut bytes=image();bytes.resize(6*1024*1024,0);let encoded=STANDARD.encode(&bytes);
        let branch=super::super::tests::indexed(&temp,&(native(crate::models::SessionKind::Codex,&encoded).to_string()+"\n"));let reference=reference(&branch);
        assert!(resolve(&branch,&reference,None,true).unwrap_err().starts_with("image_requires_download:"));
        let mut output=Vec::new();write(&branch,&reference,None,&mut output,&AtomicBool::new(false)).unwrap();assert_eq!(output,bytes);
        struct Stop<'a> {cancel:&'a AtomicBool,bytes:usize}
        impl Write for Stop<'_> {fn write(&mut self,bytes:&[u8]) -> std::io::Result<usize> {self.bytes+=bytes.len();self.cancel.store(true,Ordering::Relaxed);Ok(bytes.len())}fn flush(&mut self)->std::io::Result<()> {Ok(())}}
        let cancel=AtomicBool::new(false);let mut writer=Stop {cancel:&cancel,bytes:0};
        assert_eq!(write(&branch,&reference,None,&mut writer,&cancel).unwrap_err(),"read_canceled");assert!(writer.bytes>0 && writer.bytes<=super::super::limits::CHECKPOINT_BYTES);
    }

    #[test]
    fn opencode_native_image_parts_keep_binary_downloads_scoped_and_leave_the_database_unchanged() {
        let temp=super::super::tests::Temp::new();let path=temp.0.join("opencode.sqlite");let conn=Connection::open(&path).unwrap();
        conn.execute_batch("CREATE TABLE session(id TEXT PRIMARY KEY,revert TEXT);CREATE TABLE message(id TEXT PRIMARY KEY,session_id TEXT,time_created INTEGER,time_updated INTEGER,data TEXT);
            CREATE TABLE part(id TEXT PRIMARY KEY,session_id TEXT,message_id TEXT,time_created INTEGER,time_updated INTEGER,data TEXT);INSERT INTO session VALUES('selected',NULL);INSERT INTO session VALUES('foreign',NULL);").unwrap();
        let bytes=image();let encoded=STANDARD.encode(&bytes);
        for scope in ["selected","foreign"] {
            conn.execute("INSERT INTO message VALUES(?1,?1,1,1,?2)",params![scope,json!({"role":"user","time":{"created":1}}).to_string()]).unwrap();
            conn.execute("INSERT INTO part VALUES(?1,?1,?1,1,1,?2)",params![scope,json!({"type":"file","mime":"image/png","url":format!("data:image/png;base64,{encoded}")}).to_string()]).unwrap();
        }drop(conn);let before=std::fs::read(&path).unwrap();
        let store=super::super::store::Store::at(&temp.0).unwrap();let source=super::super::native_database::register(&store,path.clone(),"selected").unwrap();
        super::super::native_database::scan(&store,&source,&AtomicBool::new(false)).unwrap();
        let branch=Branch {store,sources:vec![source],generation:"branch".into(),kind:crate::models::SessionKind::Opencode,native_id:"selected".into(),error:None,scope:"selected".into(),parent:None};
        let reference=reference(&branch);assert_eq!(resolve(&branch,&reference,None,true).unwrap().data,encoded);
        assert_eq!(branch.page(None,None,None).unwrap()["knownRows"],1);assert_eq!(std::fs::read(path).unwrap(),before);
    }

    #[test]
    fn header_limits_cover_png_gif_jpeg_and_webp_without_decoding_pixels() {
        let mut png=b"\x89PNG\r\n\x1a\n".to_vec();png.resize(24,0);png[16..20].copy_from_slice(&123u32.to_be_bytes());png[20..24].copy_from_slice(&456u32.to_be_bytes());assert_eq!(dimensions(&png),Some((123,456)));
        assert_eq!(dimensions(b"GIF89a\x7b\x00\xc8\x00"),Some((123,200)));
        assert_eq!(dimensions(b"\xff\xd8\xff\xc0\x00\x08\x08\x01\xc8\x00\x7b\x00"),Some((123,456)));
        let mut webp=b"RIFF\x00\x00\x00\x00WEBPVP8X".to_vec();webp.resize(30,0);webp[24]=122;webp[27]=199;assert_eq!(dimensions(&webp),Some((123,200)));
        assert_eq!(dimensions(b"Unknown image"),None);assert_eq!(data_url("data:image/png;base64,YQ=="),Some(("image/png",22)));
    }
    #[test]
    fn owned_previews_keep_original_actions_and_reject_unbounded_pixel_dimensions() {
        let encoded=STANDARD.encode(image());let original=crate::agent::chat::protocol::ChatImage {mime_type:"image/png".into(),data:encoded.clone()};
        assert_eq!(resolve_owned(original.clone(),"original").unwrap(),original);
        assert_eq!(resolve_owned(original.clone(),"view").unwrap(),original);
        let preview=resolve_owned(original,"preview").unwrap();assert_ne!(preview.data,encoded);
        let decoded=image::load_from_memory(&STANDARD.decode(preview.data).unwrap()).unwrap();assert_eq!((decoded.width(),decoded.height()),(256,171));
        let mut oversized=b"\x89PNG\r\n\x1a\n".to_vec();oversized.resize(24,0);oversized[16..20].copy_from_slice(&10_000u32.to_be_bytes());oversized[20..24].copy_from_slice(&10_000u32.to_be_bytes());
        let original=crate::agent::chat::protocol::ChatImage {mime_type:"image/png".into(),data:STANDARD.encode(oversized)};
        for purpose in ["preview","view"] {assert!(resolve_owned(original.clone(),purpose).unwrap_err().starts_with("image_requires_download:"));}
        assert_eq!(resolve_owned(original.clone(),"original").unwrap(),original);
    }
}

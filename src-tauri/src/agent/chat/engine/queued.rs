//! Waiting submissions retain verified original references instead of duplicate image and text bodies.

use super::*;
use crate::agent::history_store::{live,projection::{Checkpoint,Field}};

const PREVIEW:usize=1024;
const QUEUE_BYTES:usize=2*1024*1024;
const QUEUE_ITEMS:usize=128;
const ORIGINAL_BYTES:u64=256*1024*1024;

#[derive(Clone,Debug)]
struct Content {field:Field,pieces:Vec<Checkpoint>}
#[derive(Clone,Debug)]
pub(super) struct Original {frame:live::Frame,text:Content,images:Vec<(String,Content)>,bytes:usize,shell_command:Option<String>}
impl Original {
    pub(super) fn text_deferred(&self) -> bool {self.text.field.decoded_bytes>PREVIEW as u64}
    pub(super) fn revision(&self) -> String {format!("{}:{}:{}",self.frame.reference,self.frame.start,self.frame.bytes)}
    pub(super) fn shell_command(&self) -> Option<&str> {self.shell_command.as_deref()}
}

fn read(original:&Original,content:&Content,limit:u64,cancel:&AtomicBool) -> Result<String,String> {
    if cancel.load(Ordering::Relaxed) {return Err("read_canceled".into());}
    if content.field.decoded_bytes>limit {return Err("detail_requires_paging: queued content exceeds the requested read budget".into());}
    let mut text=String::with_capacity(content.field.decoded_bytes as usize);
    for piece in &content.pieces {text.push_str(&live::read_checkpoint(&original.frame,piece,cancel)?);}
    if text.len() as u64!=content.field.decoded_bytes {return Err("source_changed: queued original checkpoints no longer cover the complete field".into());}
    Ok(text)
}

fn content(fields:&[Field],pieces:&mut HashMap<String,Vec<Checkpoint>>,path:&str) -> Result<Content,String> {
    let field=fields.iter().find(|field|field.path==path && field.encoding=="string").cloned()
        .ok_or("detail_unavailable: queued original field is unavailable")?;
    Ok(Content {field,pieces:pieces.remove(path).unwrap_or_default()})
}

pub(super) fn save(app:&AppCtx,session:&str,proc:&Arc<ChatProcess>,mut item:QueuedMessage) -> Result<QueuedMessage,String> {
    let frame=bounded::proposal(app,session,proc,&item.id,&item.text,&item.images,false)?;
    let Some(frame)=frame else {return Ok(item);};let mut pieces:HashMap<String,Vec<Checkpoint>>=HashMap::new();let mut bytes=0usize;
    let record=live::project_frame(&frame,&proc.released,&mut |checkpoint| {
        if checkpoint.path=="/text" || (checkpoint.path.starts_with("/images/") && checkpoint.path.ends_with("/data")) {
            bytes+=checkpoint.path.len()+checkpoint.hash.len()+std::mem::size_of::<Checkpoint>()+64;
            if bytes>QUEUE_BYTES {return Err("storage_pressure: queued original metadata exceeds the queue memory budget".into());}
            pieces.entry(checkpoint.path.clone()).or_default().push(checkpoint);
        }Ok(())
    })?;
    if record.value["type"]!="proposal" || record.value["id"]!=item.id {return Err("source_changed: queued proposal identity changed".into());}
    let text=content(&record.fields,&mut pieces,"/text")?;let mut images=Vec::with_capacity(item.images.len());
    let shell_command=if item.images.is_empty() {super::super::shell::parse_context(&item.text).map(|context|{
        let mut command=context.command;let mut end=command.len().min(PREVIEW);while !command.is_char_boundary(end) {end-=1;}command.truncate(end);command
    })} else {None};
    for (index,image) in item.images.iter_mut().enumerate() {
        images.push((image.mime_type.clone(),content(&record.fields,&mut pieces,&format!("/images/{index}/data"))?));image.data.clear();image.data.shrink_to_fit();
    }
    let mut end=item.text.len().min(PREVIEW);while !item.text.is_char_boundary(end) {end-=1;}item.text.truncate(end);item.text.shrink_to_fit();
    let bytes=bytes+frame.reference.len()+frame.path.as_os_str().len()+1024+images.len()*512;
    item.original=Some(Arc::new(Original {frame,text,images,bytes,shell_command}));Ok(item)
}

pub(super) fn charge(item:&QueuedMessage) -> usize {
    item.id.len()+item.text.len()+std::mem::size_of::<QueuedMessage>()+128
        +item.images.iter().map(|image|image.mime_type.len()+image.data.len()+128).sum::<usize>()
        +item.original.as_ref().map_or(0,|original|original.bytes)
}
pub(super) fn reserve(waiting:&[QueuedMessage],item:&QueuedMessage,replacing:Option<&str>) -> Result<(),String> {
    let mut count=1;let mut bytes=charge(item);
    for previous in waiting {if replacing==Some(previous.id.as_str()) {continue;}count+=1;bytes=bytes.saturating_add(charge(previous));}
    if count>QUEUE_ITEMS || bytes>QUEUE_BYTES {return Err("storage_pressure: the waiting queue has reached its memory budget; remove a waiting message before adding another".into());}
    Ok(())
}

pub(super) fn materialize(item:&QueuedMessage,cancel:&AtomicBool) -> Result<QueuedMessage,String> {
    let Some(original)=&item.original else {return Ok(item.clone());};
    let text=read(original,&original.text,ORIGINAL_BYTES,cancel)?;let mut images=Vec::with_capacity(original.images.len());
    for (mime,content) in &original.images {images.push(ChatImage {mime_type:mime.clone(),data:read(original,content,4*((crate::command_core::MAX_IMAGE_BYTES as u64+2)/3),cancel)?});}
    Ok(QueuedMessage {id:item.id.clone(),text,images,original:None})
}

pub(super) fn image(item:&QueuedMessage,index:usize,cancel:&AtomicBool) -> Result<ChatImage,String> {
    if let Some(original)=&item.original {
        let (mime,content)=original.images.get(index).ok_or("detail_unavailable: queued attachment is unavailable")?;
        return Ok(ChatImage {mime_type:mime.clone(),data:read(original,content,4*((crate::command_core::MAX_IMAGE_BYTES as u64+2)/3),cancel)?});
    }item.images.get(index).cloned().ok_or_else(||"detail_unavailable: queued attachment is unavailable".into())
}

pub(super) fn frame_image(frame:&live::Frame,id:&str,index:usize,cancel:&AtomicBool) -> Result<ChatImage,String> {
    if index>=20 {return Err("detail_unavailable: attachment index is outside this submission".into());}
    let path=format!("/images/{index}/data");let mut pieces=Vec::new();let mut bytes=0;
    let record=live::project_frame(frame,cancel,&mut |piece| {
        if piece.path==path {bytes+=piece.path.len()+piece.hash.len()+std::mem::size_of::<Checkpoint>()+64;
            if bytes>64*1024 {return Err("detail_requires_paging: attachment metadata exceeds its read budget".into());}pieces.push(piece);}
        Ok(())
    })?;
    if record.value["type"]!="dispatch" || record.value["id"]!=id {return Err("source_changed: submitted attachment identity changed".into());}
    let field=record.fields.iter().find(|field|field.path==path).ok_or("detail_unavailable: submitted attachment original is unavailable")?;
    if field.decoded_bytes>4*((crate::command_core::MAX_IMAGE_BYTES as u64+2)/3) {return Err("image_requires_download: image exceeds the original read budget".into());}
    let mime=record.value.pointer(&format!("/images/{index}/mimeType")).and_then(Value::as_str).filter(|mime|mime.len()<=100)
        .ok_or("detail_unavailable: submitted attachment media type is unavailable")?;
    let mut data=String::with_capacity(field.decoded_bytes as usize);
    for piece in pieces {data.push_str(&live::read_checkpoint(frame,&piece,cancel)?);}
    if data.len() as u64!=field.decoded_bytes {return Err("source_changed: submitted attachment original is incomplete".into());}
    Ok(ChatImage {mime_type:mime.into(),data})
}

pub(super) fn piece(item:&QueuedMessage,expected:Option<&str>,sequence:u64,cancel:&AtomicBool) -> Result<Value,String> {
    let Some(original)=&item.original else {
        if sequence!=0 {return Err("cursor_expired: queued text cursor is outside this message".into());}
        if item.text.len()>crate::agent::history_store::limits::CHECKPOINT_BYTES {return Err("detail_requires_paging: queued message requires the bounded receive contract".into());}
        return Ok(json!({"id":item.id,"text":item.text,"contentRevision":item.id,"chunkSequence":0,"eof":true,"sizeBytes":item.text.len()}));
    };
    let revision=original.revision();
    if expected.is_some_and(|expected|expected!=revision) {return Err("source_changed: queued text was edited".into());}
    let size=original.text.field.decoded_bytes;
    let text=if size==0 && sequence==0 {String::new()} else {
        let piece=original.text.pieces.get(sequence as usize).ok_or("cursor_expired: queued text cursor is outside this message")?;
        live::read_checkpoint(&original.frame,piece,cancel)?
    };
    let value=json!({"id":item.id,"text":text,"contentRevision":revision,"chunkSequence":sequence,
        "eof":sequence+1>=original.text.pieces.len() as u64,"sizeBytes":size});
    if serde_json::to_vec(&value).map_err(|e|e.to_string())?.len()>crate::agent::history_store::limits::DETAIL_BYTES {
        return Err("unsupported_record: queued text piece exceeds its wire budget".into());
    }Ok(value)
}

pub(super) fn dispatch(app:&AppCtx,session:&str,proc:&Arc<ChatProcess>,item:&QueuedMessage) -> Result<(),DispatchFailure> {
    // Only the one submission being dispatched is materialized. Waiting messages retain no payload copies.
    static DISPATCH:std::sync::OnceLock<Mutex<()>>=std::sync::OnceLock::new();
    let _dispatch=DISPATCH.get_or_init(||Mutex::new(())).lock().map_err(|_|DispatchFailure::from("Queued dispatch reader is unavailable".to_string()))?;
    let input=materialize(item,&proc.released).map_err(DispatchFailure::from)?;
    super::dispatch(app,session,proc,&input.text,&input.images,Some(&input.id))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn waiting_originals_are_paged_verified_and_restored_without_payload_copies() {
        let app=super::super::tests::ctx("queue-originals");let mut proc=super::super::tests::inert_process(SessionKind::Claude);
        Arc::get_mut(&mut proc).unwrap().bounded_protocol=true;
        let text="雪😀 full waiting original\\\"\n".repeat(5000);let image="AQID".repeat(300_000);
        let item=save(&app,"scope",&proc,QueuedMessage {id:"waiting".into(),text:text.clone(),images:vec![ChatImage {mime_type:"image/png".into(),data:image.clone()}],original:None}).unwrap();
        assert!(item.text.len()<=PREVIEW);assert!(item.images[0].data.is_empty());assert!(charge(&item)<64*1024);
        let mut result=String::new();let mut revision=None;
        for sequence in 0.. {
            let part=piece(&item,revision.as_deref(),sequence,&proc.released).unwrap();assert!(part.to_string().len()<=crate::agent::history_store::limits::DETAIL_BYTES);
            revision=part["contentRevision"].as_str().map(str::to_owned);result.push_str(part["text"].as_str().unwrap());if part["eof"]==true {break;}
        }assert_eq!(result,text);
        let complete=materialize(&item,&proc.released).unwrap();assert_eq!(complete.text,text);assert_eq!(complete.images[0].data,image);
        assert_eq!(self::image(&item,0,&proc.released).unwrap().data,image);
        assert!(piece(&item,Some("edited"),0,&proc.released).unwrap_err().starts_with("source_changed:"));
        let original=item.original.as_ref().unwrap();let raw=std::fs::read(&original.frame.path).unwrap();let mut changed=raw.clone();
        let offset=original.text.field.start as usize;changed[offset]=if changed[offset]==b'x' {b'y'} else {b'x'};std::fs::write(&original.frame.path,changed).unwrap();
        assert!(materialize(&item,&proc.released).unwrap_err().contains("source_changed:"));
        std::fs::write(&original.frame.path,raw).unwrap();proc.released.store(true,Ordering::Relaxed);
        assert_eq!(materialize(&item,&proc.released).unwrap_err(),"read_canceled");
        proc.child.lock().unwrap().wait().unwrap();let directory=app.data_dir().unwrap();drop(item);drop(proc);drop(app);std::fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn queued_dispatch_restores_the_exact_saved_input_and_failed_delivery_keeps_it_waiting() {
        let app=super::super::tests::ctx("queue-delivery-originals");let (mut proc,stdout)=super::super::tests::cat_process();
        Arc::get_mut(&mut proc).unwrap().bounded_protocol=true;proc.ready.store(true,Ordering::Relaxed);
        let text="Exact queued original 雪😀\\\"\n".repeat(5000);let image="AQID".repeat(100_000);
        let item=save(&app,"scope",&proc,QueuedMessage {id:"queued".into(),text:text.clone(),images:vec![ChatImage {mime_type:"image/png".into(),data:image.clone()}],original:None}).unwrap();
        let reader=std::thread::spawn(move||{
            let mut text=String::new();BufReader::new(stdout).read_line(&mut text).unwrap();serde_json::from_str::<Value>(&text).unwrap()
        });
        self::dispatch(&app,"scope",&proc,&item).unwrap_or_else(|failure|panic!("{}",failure.message));
        let mut received=reader.join().unwrap();let native=received.as_object_mut().unwrap().remove("uuid").unwrap();
        assert!(uuid::Uuid::parse_str(native.as_str().unwrap()).is_ok());
        assert!(received==protocol::user_message(&text,&[ChatImage {mime_type:"image/png".into(),data:image.clone()}]),"The queued input changed on the provider pipe");
        let row=proc.timeline.lock().unwrap().get("queued").cloned().unwrap();
        assert!(matches!(row,ChatRow::User {text,images,..} if text.len()<=crate::agent::history_store::limits::PREVIEW_BYTES && images[0].data.is_empty()));
        let frame=proc.runtime_originals.lock().unwrap().get("queued").cloned().unwrap();
        assert!(frame_image(&frame,"queued",0,&proc.released).unwrap().data==image);
        proc.stdin.lock().unwrap().take();proc.turn.lock().unwrap().running=false;proc.turn.lock().unwrap().waiting.push(item.clone());
        start_waiting_message(&app,"scope",&proc);
        assert!(!proc.turn.lock().unwrap().running);assert_eq!(proc.turn.lock().unwrap().waiting[0].id,"queued");
        assert!(proc.turn.lock().unwrap().waiting[0].original.is_some());
        let original=materialize(&proc.turn.lock().unwrap().waiting[0],&proc.released).unwrap();assert_eq!(original.text,text);
        let _=proc.child.lock().unwrap().kill();proc.child.lock().unwrap().wait().unwrap();
        let directory=app.data_dir().unwrap();drop(item);drop(proc);drop(app);std::fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn waiting_budget_counts_original_checkpoint_metadata_and_replacements() {
        let item=QueuedMessage {id:"waiting".into(),text:"x".repeat(20_000),images:vec![],original:None};
        let items=vec![item.clone();100];assert!(reserve(&items,&item,None).is_ok());
        let items=vec![item.clone();128];assert!(reserve(&items,&item,None).unwrap_err().starts_with("storage_pressure:"));
        assert!(reserve(&items,&item,Some("waiting")).is_ok());
        let item=QueuedMessage {text:"x".repeat(QUEUE_BYTES),..item};assert!(reserve(&[],&item,None).is_err());
    }
}

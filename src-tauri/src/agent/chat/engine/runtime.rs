//! Reconstructible runtime presentation has independent byte, row, and streaming-buffer budgets.

use super::*;

pub(super) const ROWS:usize=240;
pub(super) const ROW_BYTES:usize=8*1024*1024;
const BUFFER_BYTES:usize=2*1024*1024;
const TEXT_BYTES:usize=crate::agent::history_store::limits::PREVIEW_BYTES;
const CHILD_BYTES:usize=2*1024*1024;

pub(super) fn bound(proc:&ChatProcess,truncated:bool) {
    if !proc.bounded_protocol {return;}
    let retained={let mut timeline=proc.timeline.lock().unwrap();timeline.bounded=true;timeline.bound_rows(truncated);
        timeline.at.keys().cloned().collect::<HashSet<_>>()};
    proc.runtime_originals.lock().unwrap().retain(|id,_|retained.contains(id));
    let current=proc.current_message.lock().unwrap().clone();
    proc.frame_blocks.lock().unwrap().retain(|message,_|current.as_deref()==Some(message.as_str())
        || retained.iter().any(|id|id.strip_prefix(message).is_some_and(|suffix|suffix.starts_with(':'))));
    let mut buffers=proc.buffers.lock().unwrap();
    for text in buffers.values_mut() {shorten(text,TEXT_BYTES);}
    while buffers.len()>256 || buffers.iter().map(|(id,text)|id.len()+text.capacity()+128).sum::<usize>()>BUFFER_BYTES {
        let Some(old)=buffers.keys().next().cloned() else {break;};buffers.remove(&old);
    }
}

pub(super) fn user_row(proc:&ChatProcess,id:String,text:&str,images:&[ChatImage],at:i64,frame:Option<crate::agent::history_store::live::Frame>) -> ChatRow {
    if !proc.bounded_protocol {return super::user_row(id,text,images.to_vec(),Some(at));}
    if let Some(frame)=frame {proc.runtime_originals.lock().unwrap().insert(id.clone(),frame);}
    let mut timeline=proc.timeline.lock().unwrap();timeline.bounded=true;
    if text.len()>TEXT_BYTES || !images.is_empty() {timeline.deferred_rows.insert(id.clone());}drop(timeline);
    let preview=crate::agent::history_store::adapter::preview(text,TEXT_BYTES);
    super::user_row(id,&preview,images.iter().take(20).map(|image|ChatImage {mime_type:image.mime_type.clone(),data:String::new()}).collect(),Some(at))
}

fn shorten(text:&mut String,bytes:usize) -> bool {
    if text.len()<=bytes {return false;}
    let mut end=bytes;while !text.is_char_boundary(end) {end-=1;}text.truncate(end);text.shrink_to_fit();true
}
fn value_bytes(value:&Value) -> usize {
    std::mem::size_of::<Value>()+match value {
        Value::String(text)=>text.capacity(),
        Value::Array(values)=>values.capacity()*std::mem::size_of::<Value>()+values.iter().map(value_bytes).sum::<usize>(),
        Value::Object(values)=>values.iter().map(|(key,value)|key.capacity()+96+value_bytes(value)).sum::<usize>(),
        _=>0,
    }
}
pub(super) fn bytes(row:&ChatRow) -> usize {
    let strings=match row {
        ChatRow::User {text,images,..}=>text.capacity()+images.capacity()*std::mem::size_of::<ChatImage>()
            +images.iter().map(|image|image.data.capacity()+image.mime_type.capacity()+64).sum::<usize>(),
        ChatRow::Assistant {text,model,..}=>text.capacity()+model.as_ref().map_or(0,String::capacity),
        ChatRow::Reasoning {text,..}|ChatRow::Command {text,..}=>text.capacity(),
        ChatRow::Tool {name,input,output,subagent,children,..}=>name.capacity()+value_bytes(input)+output.as_ref().map_or(0,String::capacity)
            +subagent.as_ref().map_or(0,|info|info.title.as_ref().map_or(0,String::capacity)+info.description.as_ref().map_or(0,String::capacity)
                +info.model.as_ref().map_or(0,String::capacity)+std::mem::size_of::<SubagentInfo>())
            +children.capacity()*std::mem::size_of::<ChatRow>()+children.iter().map(bytes).sum::<usize>(),
        ChatRow::Shell {command,stdout,stderr,source_text,..}=>command.capacity()+stdout.capacity()+stderr.capacity()+source_text.capacity(),
        ChatRow::Error {message,..}|ChatRow::Notice {message,..}=>message.capacity(),
        ChatRow::Compaction {trigger,..}=>trigger.as_ref().map_or(0,String::capacity),
        ChatRow::Wake {summary,..}=>summary.capacity(),
    };
    // Include owned metadata and conservative copies for flush, JSON conversion, and IPC staging.
    (strings+row.id().len()+std::mem::size_of::<ChatRow>()+512).saturating_mul(4)
}
pub(super) fn summary(row:&mut ChatRow) -> bool {
    fn visit(row:&mut ChatRow,depth:usize,remaining:&mut usize) -> bool {
        *remaining=remaining.saturating_sub(1);let text_limit=if depth==0 {TEXT_BYTES} else {1024};
        match row {
            ChatRow::User {text,images,..}=> {
                let mut changed=shorten(text,text_limit);
                if images.len()>20 {images.truncate(20);images.shrink_to_fit();changed=true;}
                for image in images {changed|=shorten(&mut image.mime_type,100);
                    if !image.data.is_empty() {image.data.clear();image.data.shrink_to_fit();changed=true;}}
                changed
            },
            ChatRow::Assistant {text,model,..}=>{let mut changed=shorten(text,text_limit);if let Some(model)=model {changed|=shorten(model,256);}changed},
            ChatRow::Reasoning {text,..}|ChatRow::Command {text,..}=>shorten(text,text_limit),
            ChatRow::Tool {name,input,output,subagent,children,..}=> {
                let mut changed=shorten(name,256);
                if name!="AskUserQuestion" {let next=crate::agent::history_store::adapter::short_input(input);changed|=*input!=next;*input=next;}
                if let Some(output)=output {changed|=shorten(output,512);}
                if let Some(facts)=subagent {for text in [&mut facts.title,&mut facts.description,&mut facts.model].into_iter().flatten() {changed|=shorten(text,1024);}}
                let mut keep=0;
                if depth<8 {for child in children.iter_mut() {if *remaining==0 {break;}changed|=visit(child,depth+1,remaining);keep+=1;}}
                if children.len()>keep {children.truncate(keep);children.shrink_to_fit();changed=true;}
                changed
            },
            ChatRow::Shell {command,stdout,stderr,source_text,..}=> {
                let changed=shorten(command,1024)|shorten(stdout,4096)|shorten(stderr,4096)|!source_text.is_empty();
                source_text.clear();source_text.shrink_to_fit();changed
            },
            ChatRow::Error {message,..}|ChatRow::Notice {message,..}=>shorten(message,text_limit),
            ChatRow::Compaction {trigger,..}=>trigger.as_mut().is_some_and(|text|shorten(text,256)),
            ChatRow::Wake {summary,..}=>shorten(summary,text_limit),
        }
    }
    visit(row,0,&mut 16)
}

pub(super) fn append(proc:&ChatProcess,buffers:&mut HashMap<String,String>,id:&str,text:&str) -> String {
    if !proc.bounded_protocol {let buffer=buffers.entry(id.into()).or_default();buffer.push_str(text);return buffer.clone();}
    // These are previews only. Every complete delta is already durable in the original journal.
    if !buffers.contains_key(id) && buffers.len()>=256 {if let Some(old)=buffers.keys().next().cloned() {buffers.remove(&old);}}
    let buffer=buffers.entry(id.into()).or_default();
    let left=TEXT_BYTES.saturating_sub(buffer.len());let mut end=text.len().min(left);while !text.is_char_boundary(end) {end-=1;}
    buffer.push_str(&text[..end]);let result=buffer.clone();
    while buffers.iter().map(|(id,text)|id.len()+text.capacity()+128).sum::<usize>()>BUFFER_BYTES {
        let Some(old)=buffers.keys().find(|key|key.as_str()!=id).cloned() else {break;};buffers.remove(&old);
    }result
}

pub(super) fn pending_child(pending:&mut HashMap<String,Vec<(String,Value)>>,thread:&str,method:&str,value:Value) {
    fn charge(pending:&HashMap<String,Vec<(String,Value)>>) -> usize {
        pending.iter().map(|(id,events)|id.capacity()+128+events.capacity()*std::mem::size_of::<(String,Value)>()
            +events.iter().map(|(method,value)|method.capacity()+value_bytes(value)).sum::<usize>()).sum::<usize>()
    }
    if thread.len()>200 || method.len()>200 || value_bytes(&value)>CHILD_BYTES {return;}
    if !pending.contains_key(thread) && pending.len()>=32 {return;}
    let events=pending.entry(thread.into()).or_default();
    if events.len()>=128 {events.remove(0);}
    events.push((method.into(),value));
    // This backlog contains presentation events, never permission/control requests. Their complete
    // originals remain durable even if an as-yet-unassociated child preview is evicted here.
    while charge(pending)>CHILD_BYTES {
        let Some(key)=pending.keys().find(|key|key.as_str()!=thread).cloned().or_else(||pending.keys().next().cloned()) else {break;};
        pending.remove(&key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn timeline_eviction_bounds_rows_and_bytes_while_dirty_wire_batches_keep_every_retained_update() {
        let mut timeline=Timeline {bounded:true,..Default::default()};let text="雪😀\n\\\"".repeat(4096);
        for index in 0..2000 {
            timeline.upsert(ChatRow::Assistant {id:format!("answer-{index}"),text:text.clone(),streaming:true,model:None,at:None,duration_ms:None});
            timeline.bound_rows(false);
            assert!(timeline.rows.len()<=ROWS);assert!(timeline.retained_bytes.values().copied().sum::<usize>()<=ROW_BYTES);
        }
        assert!(timeline.dropped_rows>0);assert_eq!(timeline.at.len(),timeline.rows.len());
        assert!(timeline.deferred_rows.len()<=ROWS);assert!(timeline.row_versions.len()<=ROWS);assert!(timeline.dirty.len()<=ROWS);
        let expected=timeline.rows.iter().map(|row|row.id().to_owned()).collect::<HashSet<_>>();let mut sent=HashSet::new();
        while !timeline.dirty.is_empty() {
            let rows=timeline.take_dirty();assert!(!rows.is_empty());let values=timeline.wire_window(&rows);assert_eq!(values.len(),rows.len());
            let event=json!({"type":"rows","rows":values,"positions":rows.iter().map(|row|(row.id(),timeline.at[row.id()])).collect::<HashMap<_,_>>(),"revision":timeline.revision,"epoch":7});
            assert!(serde_json::to_vec(&event).unwrap().len()<=crate::agent::history_store::limits::PAGE_BYTES);
            for row in &rows {assert!(sent.insert(row.id().to_owned()));assert_eq!(timeline.display(row)["bodyAvailable"],true);}
        }
        assert_eq!(sent,expected);
        let mut row=ChatRow::Tool {id:"nested".into(),name:"Task".into(),input:json!({"content":"x".repeat(256*1024)}),output:Some("x".repeat(256*1024)),
            is_error:false,status:"running",subagent:None,children:(0..1000).map(|index|ChatRow::Assistant {id:format!("child-{index}"),text:text.clone(),streaming:true,model:None,at:None,duration_ms:None}).collect()};
        assert!(summary(&mut row));assert!(bytes(&row)<1024*1024);
        assert!(matches!(row,ChatRow::Tool {children,..} if children.len()<=16));
    }
    #[test]
    fn streaming_previews_bound_accumulation_and_unknown_child_bytes_before_route_registration() {
        let mut proc=super::super::tests::inert_process(SessionKind::Codex);Arc::get_mut(&mut proc).unwrap().bounded_protocol=true;
        let mut buffers=HashMap::new();let delta="雪😀\n".repeat(1024);
        for index in 0..600 {for _ in 0..4 {let text=append(&proc,&mut buffers,&format!("item-{index}"),&delta);assert!(text.len()<=TEXT_BYTES);}}
        assert!(buffers.len()<=256);assert!(buffers.iter().map(|(id,text)|id.len()+text.capacity()+128).sum::<usize>()<=BUFFER_BYTES);
        let mut children=HashMap::new();for index in 0..100 {for _ in 0..128 {
            pending_child(&mut children,&format!("thread-{index}"),"item/agentMessage/delta",json!({"text":"x".repeat(64*1024)}));
        }}
        assert!(children.len()<=32);assert!(children.values().flatten().map(|(_,value)|value_bytes(value)).sum::<usize>()<=CHILD_BYTES);
        proc.child.lock().unwrap().wait().unwrap();
    }
}

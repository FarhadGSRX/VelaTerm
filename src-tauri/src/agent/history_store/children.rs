//! Parent-scoped child histories. A client follows indexed tool rows, never a native id or a path.

use std::collections::HashSet;
use std::sync::Arc;
use rusqlite::{params,Connection,OptionalExtension};
use serde_json::{json,Value};
use crate::host::AppCtx;
use crate::models::SessionKind;
use super::{adapter,api::{self,Branch},projection::Record,source::{self,Segment},store::{digest,sql,RegisteredSource}};

pub(super) fn is_claude_child(source:&RegisteredSource) -> bool {
    source.segment.native_id.starts_with("claude-child:") && source.segment.path.parent().is_some_and(|path|path.ends_with("subagents"))
}

pub(super) fn annotate(kind:SessionKind,record:&Record,row:&mut Value) -> Result<(),String> {
    if row["kind"]!="tool" {return Ok(());}
    let (id,facts)=match kind {
        SessionKind::Claude=>{
            let result=&record.value["toolUseResult"];
            let Some(id)=result["agentId"].as_str() else {return Ok(());};
            (id,json!({"description":result["description"],"model":result["resolvedModel"],"totalTokens":result["totalTokens"],
                "toolUses":result["totalToolUseCount"],"durationMs":result["totalDurationMs"]}))
        },
        SessionKind::Opencode=>{
            let part=&record.value["part"];if part["tool"]!="task" {return Ok(());}
            let Some(id)=part.pointer("/state/metadata/sessionId").or_else(||part.pointer("/state/metadata/sessionID")).and_then(Value::as_str) else {return Ok(());};
            (id,row["subagent"].clone())
        },
        _=>return Ok(()),
    };
    if !valid(id) {return Err("unsupported_record: child identity exceeds its metadata budget".into());}
    row["_child"]=json!({"id":id,"facts":facts});Ok(())
}

fn valid(id:&str) -> bool { !id.is_empty() && id.len()<=200 && id.bytes().all(|byte|byte.is_ascii_alphanumeric() || matches!(byte,b'-'|b'_')) }

pub(super) fn public_row(mut row:Value) -> Value {
    if let Some(child)=row.as_object_mut().and_then(|object|object.remove("_child")) {
        if !matches!(row["name"].as_str(),Some("Task"|"Agent")) {return row;}
        row["childAvailable"]=json!(true);
        let mut facts=row["subagent"].as_object().cloned().unwrap_or_default();
        if let Some(values)=child["facts"].as_object() {for (key,value) in values {if !value.is_null() {facts.insert(key.clone(),value.clone());}}}
        if !facts.is_empty() {row["subagent"]=Value::Object(facts);}
    }
    row
}

pub(super) fn merge(previous:&Value,current:&mut Value) {
    if let Some(child)=previous.get("_child") {
        if current.get("_child").is_none() {current["_child"]=child.clone();}
        else if current["_child"]["id"]==child["id"] {
            let mut facts=child["facts"].as_object().cloned().unwrap_or_default();
            if let Some(updated)=current["_child"]["facts"].as_object() {for (key,value) in updated {if !value.is_null() {facts.insert(key.clone(),value.clone());}}}
            current["_child"]["facts"]=Value::Object(facts);
        }
    }
}

pub(super) fn codex_record(conn:&Connection,source:&RegisteredSource,record:&Record,item:&Value) -> Result<(),String> {
    let category=item["type"].as_str().unwrap_or_default().replace('_',"").to_ascii_lowercase();
    let (child,status,description)=if category=="subagentactivity" {
        let Some(child)=item["agent_thread_id"].as_str().or_else(||item["agentThreadId"].as_str()) else {return Ok(());};
        let status=match item["kind"].as_str().unwrap_or("started") {"completed"|"success"=>"completed","interrupted"|"canceled"|"cancelled"=>"canceled","started"|"running"=>"running",_=>"failed"};
        (child,status,item["agent_path"].as_str().or_else(||item["agentPath"].as_str()))
    } else if category=="collabagenttoolcall" {
        let Some(child)=item["receiver_thread_ids"].as_array().or_else(||item["receiverThreadIds"].as_array()).and_then(|ids|ids.iter().find_map(Value::as_str)) else {return Ok(());};
        let status=match item["status"].as_str() {Some("completed")=>"completed",Some("failed")=>"failed",Some("canceled"|"cancelled")=>"canceled",_=>"running"};
        (child,status,item["prompt"].as_str().or_else(||item["tool"].as_str()))
    } else {return Ok(());};
    if !valid(child) {return Err("unsupported_record: child identity exceeds its metadata budget".into());}
    let id=format!("child:{}",digest(format!("codex:{}:{child}",source.segment.native_id).as_bytes()));
    let mut row=json!({"id":id,"kind":"tool","name":"Task","input":{},"status":status,"isError":status=="failed",
        "detailAvailable":true,"_child":{"id":child,"facts":{"description":description,"model":item["model"]}}});
    if let Some(description)=description {row["input"]=json!({"description":adapter::preview(description,512)});}
    let previous:Option<String>=conn.query_row("SELECT summary FROM rows WHERE source_id=?1 AND generation=?2 AND id=?3",
        params![source.id,source.generation,id],|row|row.get(0)).optional().map_err(sql)?;
    if let Some(previous)=previous {let previous:Value=serde_json::from_str(&previous).map_err(|_|"index_incomplete: child summary is invalid")?;merge(&previous,&mut row);}
    adapter::write_row(conn,source,record,0,None,&row)
}

fn linked(parent:&Branch,row_id:&str) -> Result<(RegisteredSource,String),String> {
    // Resolve aliases and visibility before consulting private metadata. Knowledge of a native child
    // id does not confer access, and a row discarded by rewind cannot authorize its old descendants.
    let manifest=parent.manifest(row_id,0)?;
    if manifest["row"]["childAvailable"]!=true {return Err("detail_unavailable: this history row is not a verified child task".into());}
    let conn=parent.store.connect()?;
    let row_id=parent.resolve_row(&conn,row_id)?;
    for source in parent.sources.iter().rev() {
        let selected:Option<(String,u64,String)>=conn.query_row("SELECT json_extract(p.summary,'$._child.id'),p.record_start,r.projection FROM row_parts p
            JOIN records r ON r.source_id=p.source_id AND r.generation=p.generation AND r.start=p.record_start
            WHERE p.source_id=?1 AND p.generation=?2 AND p.row_id=?3 AND r.active=1
            AND (?4 IS NULL OR p.record_end<=?4) AND (?5 IS NULL OR p.ordinal IS NULL OR p.ordinal<?5)
            AND json_type(p.summary,'$._child.id')='text' ORDER BY p.record_start DESC LIMIT 1",
            params![source.id,source.generation,row_id,source.segment.end_byte,source.segment.end_ordinal],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).optional().map_err(sql)?;
        if let Some((id,start,projection))=selected {if valid(&id) {
            let value:Value=serde_json::from_str(&projection).map_err(|_|"index_incomplete: parent association projection is invalid")?;
            let mut fields=conn.prepare("SELECT ref,path FROM fields WHERE source_id=?1 AND generation=?2 AND record_start=?3 AND encoding='string'").map_err(sql)?;
            let mut fields=fields.query(params![source.id,source.generation,start]).map_err(sql)?;
            let mut verified_child=false;
            while let Some(field)=fields.next().map_err(sql)? {
                let reference:String=field.get(0).map_err(sql)?;let path:String=field.get(1).map_err(sql)?;
                if !super::projection::identity_path(&path) {continue;}
                let expected=value.pointer(&path).and_then(Value::as_str).ok_or("index_incomplete: parent identity projection is unavailable")?;
                let (text,_,end,size)=api::read_piece(&conn,source,&reference,0,0)?;
                if end!=size || text!=expected {return Err("source_changed: parent child association changed".into());}
                if text==id && (path=="/toolUseResult/agentId" || path.ends_with("/agent_thread_id") || path.ends_with("/agentThreadId")
                    || path.starts_with("/payload/item/receiver_thread_ids/") || path.starts_with("/payload/item/receiverThreadIds/")
                    || path.starts_with("/params/item/receiver_thread_ids/") || path.starts_with("/params/item/receiverThreadIds/")
                    || path=="/part/state/metadata/sessionId" || path=="/part/state/metadata/sessionID") {verified_child=true;}
            }
            if !verified_child {return Err("index_incomplete: parent has no verified original child identity".into());}
            return Ok((source.clone(),id));
        }}
    }
    Err("detail_unavailable: this history row has no verified child association".into())
}

fn child(parent:Branch,row:&str,seen:&mut HashSet<(String,String)>) -> Result<Branch,String> {
    let (origin,id)=linked(&parent,row)?;
    if !seen.insert((parent.kind.as_str().into(),id.clone())) {return Err("index_incomplete: child history contains a cycle".into());}
    let (sources,native,error)=match parent.kind {
        SessionKind::Claude=>{
            let directory=if is_claude_child(&origin) {origin.segment.path.parent().unwrap().to_owned()}
                else {origin.segment.path.with_extension("").join("subagents")};
            let path=directory.join(format!("agent-{id}.jsonl"));
            if !std::fs::symlink_metadata(&directory).is_ok_and(|metadata|metadata.is_dir()) {return Err("source_missing: child recording directory is unavailable".into());}
            let metadata=std::fs::symlink_metadata(&path).map_err(|_|"source_missing: child recording is unavailable")?;
            if !metadata.is_file() {return Err("source_missing: child recording is not a regular file".into());}
            let native=format!("claude-child:{}",digest(format!("{}:{id}",directory.to_string_lossy()).as_bytes()));
            let source=parent.store.register(SessionKind::Claude,Segment {path,native_id:native.clone(),end_byte:None,end_ordinal:None})?;
            (vec![source],native,None)
        },
        SessionKind::Codex=>{
            let resolved=source::resolve(SessionKind::Codex,&id)?;
            // The child's own completed items form its work log; inherited ancestor intervals belong
            // to its private context and are not recursively presented under the parent's Task card.
            let mut sources=Vec::new();for segment in resolved.segments.into_iter().filter(|segment|segment.native_id==id) {
                sources.push(parent.store.register(SessionKind::Codex,segment)?);
            }(sources,id.clone(),resolved.error)
        },
        SessionKind::Opencode=>{
            (vec![super::native_database::register(&parent.store,origin.segment.path.clone(),&id)?],id.clone(),None)
        },
        _=>return Err("detail_unavailable: this provider has no child history adapter".into()),
    };
    let scope=format!("child:{}",digest(format!("{}:{}:{row}",parent.scope,parent.generation).as_bytes()));
    let owner=Arc::new(Parent {revision:revision(&parent)?,branch:parent.clone(),row:row.into(),child:id.clone()});
    let mut branch=api::bind(parent.store,sources,parent.kind,native,error,&scope)?;branch.parent=Some(owner);Ok(branch)
}

pub(super) struct Parent {branch:Branch,row:String,child:String,revision:String}
fn revision(branch:&Branch) -> Result<String,String> {
    let conn=branch.store.connect()?;let mut signature=Vec::new();
    for source in &branch.sources {
        let (generation,revision):(String,String)=conn.query_row("SELECT generation,branch_revision FROM sources WHERE id=?1",[&source.id],|row|Ok((row.get(0)?,row.get(1)?))).map_err(sql)?;
        if generation!=source.generation {return Err("source_changed: parent history generation changed".into());}
        signature.push(json!([source.id,generation,revision,source.segment.end_byte,source.segment.end_ordinal]));
    }
    Ok(digest(serde_json::to_string(&signature).unwrap().as_bytes()))
}
pub(super) fn validate_parent(branch:&Branch) -> Result<(),String> {
    if let Some(parent)=&branch.parent {
        if revision(&parent.branch)?!=parent.revision {return Err("cursor_expired: parent history branch changed".into());}
        let (_,child)=linked(&parent.branch,&parent.row)?;
        if child!=parent.child {return Err("source_changed: parent child association changed".into());}
    }Ok(())
}

pub fn descend(mut branch:Branch,path:&[String]) -> Result<Branch,String> {
    if path.len()>16 || path.iter().any(|row|row.is_empty() || row.len()>200) {return Err("unsupported_record: child history path exceeds its metadata budget".into());}
    let mut seen=HashSet::from([(branch.kind.as_str().into(),branch.native_id.clone())]);
    for row in path {branch=child(branch,row,&mut seen)?;}Ok(branch)
}
pub fn prepare(app:&AppCtx,session:&str,path:&[String]) -> Result<Branch,String> {descend(api::prepare(app,session)?,path)}

pub fn path(args:&Value) -> Result<Vec<String>,String> {
    let Some(value)=args.get("childPath").filter(|value|!value.is_null()) else {return Ok(Vec::new());};
    let values=value.as_array().filter(|values|values.len()<=16).ok_or("unsupported_record: child history path exceeds its metadata budget")?;
    values.iter().map(|value|value.as_str().filter(|row|!row.is_empty() && row.len()<=200).map(str::to_owned)
        .ok_or_else(||"unsupported_record: child history row identity is invalid".into())).collect()
}

pub fn task(app:&AppCtx,session:&str,task:&str) -> Result<(Branch,Vec<String>),String> {
    if !valid(task) {return Err("detail_unavailable: child task identity is invalid".into());}
    let root=api::prepare(app,session)?;let conn=root.store.connect()?;
    for source in root.sources.iter().rev() {
        let row:Option<String>=conn.query_row("SELECT p.row_id FROM row_parts p JOIN records r
            ON r.source_id=p.source_id AND r.generation=p.generation AND r.start=p.record_start
            WHERE p.source_id=?1 AND p.generation=?2 AND json_extract(p.summary,'$._child.id')=?3 AND r.active=1
            AND (?4 IS NULL OR p.record_end<=?4) AND (?5 IS NULL OR p.ordinal IS NULL OR p.ordinal<?5)
            ORDER BY p.record_start DESC LIMIT 1",params![source.id,source.generation,task,source.segment.end_byte,source.segment.end_ordinal],|row|row.get(0)).optional().map_err(sql)?;
        if let Some(row)=row {let path=vec![row];return Ok((descend(root,&path)?,path));}
    }
    Err("index_busy: this task has no indexed parent association yet".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    fn root(temp:&super::super::tests::Temp,raw:&str) -> Branch {
        let path=temp.0.join("root.jsonl");std::fs::write(&path,raw).unwrap();let store=super::super::store::Store::at(&temp.0).unwrap();
        let source=store.register(SessionKind::Claude,Segment {path,native_id:"parent".into(),end_byte:None,end_ordinal:None}).unwrap();
        store.scan(SessionKind::Claude,&source,&AtomicBool::new(false),false).unwrap();
        api::bind(store,vec![source],SessionKind::Claude,"parent".into(),None,"parent-scope").unwrap()
    }
    fn task(id:&str,parent:Option<&str>,call:&str,child:&str,sidechain:bool) -> String {
        [json!({"type":"assistant","uuid":id,"parentUuid":parent,"isSidechain":sidechain,"message":{"content":[{"type":"tool_use","id":call,"name":"Agent","input":{"description":"Delegated task"}}]}}),
            json!({"type":"user","uuid":format!("{id}-result"),"parentUuid":id,"isSidechain":sidechain,"toolUseResult":{"agentId":child,"description":"Verified child","totalTokens":719},"message":{"content":[{"type":"tool_result","tool_use_id":call,"content":"Done"}]}})]
            .iter().map(|row|format!("{row}\n")).collect()
    }
    fn indexed(branch:Branch) -> Branch {
        for source in &branch.sources {branch.store.scan(branch.kind,source,&AtomicBool::new(false),false).unwrap();}branch
    }

    #[test]
    fn children_are_lazy_paged_and_authorized_by_each_visible_parent_row() {
        let temp=super::super::tests::Temp::new();let directory=temp.0.join("root/subagents");std::fs::create_dir_all(&directory).unwrap();
        let raw=task("root",None,"call","child",false);let root=root(&temp,&raw);let page=root.page(None,None,None).unwrap();
        assert_eq!(page["rows"].as_array().unwrap().len(),1);let row=&page["rows"][0];let parent_id=row["id"].as_str().unwrap().to_owned();
        assert_eq!(row["childAvailable"],true);assert_eq!(row["subagent"]["totalTokens"],719);assert!(row.get("_child").is_none());
        assert_eq!(root.store.connect().unwrap().query_row("SELECT count(*) FROM sources",[],|row|row.get::<_,u64>(0)).unwrap(),1);
        assert!(descend(root.clone(),&["child".into()]).err().unwrap().starts_with("detail_unavailable:"));
        assert!(descend(root.clone(),&[parent_id.clone()]).err().unwrap().starts_with("source_missing:"));
        let mut child_raw=String::new();let mut parent=None;for index in 0..160 {
            let id=format!("step-{index}");child_raw.push_str(&format!("{}\n",json!({"type":"assistant","uuid":id,"parentUuid":parent,"isSidechain":true,"message":{"content":[{"type":"text","text":format!("Child step {index}")}]}})));parent=Some(id);
        }
        child_raw.push_str(&task("nested",parent.as_deref(),"nested-call","grandchild",true));
        std::fs::write(directory.join("agent-child.jsonl"),&child_raw).unwrap();
        let long="Original grandchild 雪😀\n".repeat(30_000);let grandchild_raw=format!("{}\n",json!({"type":"assistant","uuid":"grandchild-answer","isSidechain":true,"message":{"content":[{"type":"text","text":long}]}}));
        std::fs::write(directory.join("agent-grandchild.jsonl"),&grandchild_raw).unwrap();
        let child=indexed(descend(root.clone(),&[parent_id.clone()]).unwrap());let page=child.page(None,None,None).unwrap();
        assert_eq!(page["rows"].as_array().unwrap().len(),60);assert_eq!(page["knownRows"],161);assert_eq!(page["hasMore"],true);
        assert!(page.to_string().len()<super::super::limits::PAGE_BYTES);let nested=page["rows"].as_array().unwrap().iter().find(|row|row["kind"]=="tool").unwrap()["id"].as_str().unwrap().to_owned();
        assert_eq!(child.store.connect().unwrap().query_row("SELECT count(*) FROM sources",[],|row|row.get::<_,u64>(0)).unwrap(),2);
        assert!(descend(root.clone(),&[nested.clone()]).is_err());
        let grandchild=indexed(descend(root.clone(),&[parent_id.clone(),nested]).unwrap());let page=grandchild.page(None,None,None).unwrap();assert_eq!(page["rows"][0]["bodyAvailable"],true);
        let field=grandchild.manifest(page["rows"][0]["id"].as_str().unwrap(),0).unwrap()["fields"][0].clone();let reference=field["detailRef"].as_str().unwrap();
        assert!(root.detail_page(reference,None,None).is_err());assert!(child.detail_page(reference,None,None).is_err());
        let first=grandchild.detail_page(reference,None,None).unwrap();let cursor=first["nextCursor"].as_str().unwrap();
        let other=Branch {scope:"another-parent".into(),..grandchild.clone()};assert!(other.detail_page(reference,None,Some(cursor)).is_err());
        let mut original=first["text"].as_str().unwrap().to_owned();let mut cursor=Some(cursor.to_owned());while let Some(next)=cursor {
            let part=grandchild.detail_page(reference,None,Some(&next)).unwrap();original.push_str(part["text"].as_str().unwrap());cursor=part["nextCursor"].as_str().map(str::to_owned);
        }assert_eq!(original,long);
        root.store.connect().unwrap().execute("UPDATE records SET active=0 WHERE source_id=?1",[&root.sources[0].id]).unwrap();
        assert!(grandchild.detail_page(reference,None,None).is_err());assert!(grandchild.detail_facts(reference,None).is_err());
        assert!(child.page(None,None,None).is_err());assert!(descend(root,&[parent_id]).is_err());assert_eq!(std::fs::read_to_string(directory.join("agent-child.jsonl")).unwrap(),child_raw);
        assert_eq!(std::fs::read_to_string(directory.join("agent-grandchild.jsonl")).unwrap(),grandchild_raw);
    }

    #[test]
    fn retained_child_details_verify_the_original_parent_association_before_every_read() {
        let temp=super::super::tests::Temp::new();let directory=temp.0.join("root/subagents");std::fs::create_dir_all(&directory).unwrap();
        let original=task("root",None,"call","child",false);let root=root(&temp,&original);
        std::fs::write(directory.join("agent-child.jsonl"),format!("{}\n",json!({"type":"assistant","uuid":"answer","isSidechain":true,"message":{"content":[{"type":"text","text":"Original child"}]}}))).unwrap();
        let row=root.page(None,None,None).unwrap()["rows"][0]["id"].as_str().unwrap().to_owned();
        let child=indexed(descend(root.clone(),&[row]).unwrap());let page=child.page(None,None,None).unwrap();
        let manifest=child.manifest(page["rows"][0]["id"].as_str().unwrap(),0).unwrap();let reference=manifest["fields"][0]["detailRef"].as_str().unwrap();
        assert_eq!(child.detail(reference,None,0).unwrap()["text"],"Original child");
        let rewritten=original.replace("\"agentId\":\"child\"","\"agentId\":\"other\"");assert_eq!(rewritten.len(),original.len());assert_ne!(rewritten,original);
        std::fs::write(&root.sources[0].segment.path,&rewritten).unwrap();
        assert!(child.detail(reference,None,0).unwrap_err().starts_with("source_changed:"));
        assert!(child.page(None,None,None).is_err());
    }

    #[test]
    fn codex_child_summaries_merge_lifecycle_and_omit_inherited_context() {
        let temp=super::super::tests::Temp::new();let rows=[
            json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"Inherited private prompt"}]}}),
            json!({"type":"event_msg","payload":{"type":"item_completed","item":{"id":"start","type":"SubAgentActivity","kind":"started","agent_thread_id":"child","agent_path":"/root/checker"}}}),
            json!({"type":"event_msg","payload":{"type":"item_completed","item":{"id":"answer","type":"AgentMessage","content":"Child answer"}}}),
            json!({"type":"event_msg","payload":{"type":"item_completed","item":{"id":"finish","type":"SubAgentActivity","kind":"completed","agent_thread_id":"child"}}}),
        ];let raw=rows.iter().map(|row|format!("{row}\n")).collect::<String>();let root=super::super::tests::indexed(&temp,&raw);
        let page=root.page(None,None,None).unwrap();let task=page["rows"].as_array().unwrap().iter().find(|row|row["kind"]=="tool").unwrap();
        assert_eq!(task["childAvailable"],true);assert_eq!(task["status"],"completed");assert!(task.get("_child").is_none());
        assert!(task["subagent"]["description"].as_str().is_some_and(|description|description=="/root/checker"));
        let child=Branch {scope:"child:authorized-parent".into(),..root};let page=child.page(None,None,None).unwrap();
        assert_eq!(page["rows"].as_array().unwrap().len(),2);assert!(!page.to_string().contains("Inherited private prompt"));
        assert!(path(&json!({"childPath":vec!["row";17]})).is_err());assert!(path(&json!({"childPath":["../native"]})).is_ok());
        // A syntactically small row locator still requires a visible parent association.
        assert!(descend(child,&["../native".into()]).is_err());
    }

    #[test]
    fn child_parallel_calls_are_restored_but_rewound_prompts_stay_inactive() {
        let temp=super::super::tests::Temp::new();let directory=temp.0.join("root/subagents");std::fs::create_dir_all(&directory).unwrap();
        let root=root(&temp,&task("root",None,"call","child",false));let parent_id=root.page(None,None,None).unwrap()["rows"][0]["id"].as_str().unwrap().to_owned();
        let rows=[
            json!({"type":"user","uuid":"u","isSidechain":true,"message":{"content":"Prompt"}}),
            json!({"type":"assistant","uuid":"a","parentUuid":"u","isSidechain":true,"message":{"id":"same","content":[{"type":"tool_use","id":"one","name":"Read","input":{"file_path":"one"}}]}}),
            json!({"type":"assistant","uuid":"b","parentUuid":"a","isSidechain":true,"message":{"id":"same","content":[{"type":"tool_use","id":"two","name":"Read","input":{"file_path":"two"}}]}}),
            json!({"type":"user","uuid":"b-result","parentUuid":"b","isSidechain":true,"message":{"content":[{"type":"tool_result","tool_use_id":"two","content":"Second file"}]}}),
            json!({"type":"user","uuid":"rewound","parentUuid":"b-result","isSidechain":true,"message":{"content":"Discarded prompt"}}),
            json!({"type":"user","uuid":"a-result","parentUuid":"a","isSidechain":true,"message":{"content":[{"type":"tool_result","tool_use_id":"one","content":"First file"}]}}),
            json!({"type":"assistant","uuid":"answer","parentUuid":"a-result","isSidechain":true,"message":{"content":[{"type":"text","text":"Final answer"}]}}),
        ];let raw=rows.iter().map(|row|format!("{row}\n")).collect::<String>();std::fs::write(directory.join("agent-child.jsonl"),&raw).unwrap();
        let child=indexed(descend(root,&[parent_id]).unwrap());let page=child.page(None,None,None).unwrap();let rows=page["rows"].as_array().unwrap();
        assert!(rows.iter().any(|row|row["output"]=="First file"));assert!(rows.iter().any(|row|row["output"]=="Second file"));
        assert!(!rows.iter().any(|row|row["text"]=="Discarded prompt"));assert!(rows.iter().any(|row|row["text"]=="Final answer"));
        assert_eq!(std::fs::read_to_string(directory.join("agent-child.jsonl")).unwrap(),raw);
    }
}

//! Adapt OpenCode v2's native messages and events to the shared OpenCode timeline contract.

use std::collections::HashMap;
use serde_json::{json, Value};
use crate::agent::opencode_store::OpencodeMessage;

fn string<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or("")
}

pub fn part_id(message: &str, kind: &str, ordinal: u64) -> String {
    format!("oc2:{message}:{kind}:{ordinal}")
}

fn tool_id(message: &str, call: &str) -> String {
    format!("oc2:{message}:tool:{call}")
}

fn output(content: &Value) -> String {
    content.as_array().into_iter().flatten().filter_map(|part| {
        part.get("text").or_else(|| part.get("uri")).and_then(Value::as_str)
    }).collect::<Vec<_>>().join("\n")
}

fn token_total(info: &mut Value) {
    let Some(tokens)=info.get_mut("tokens").filter(|v|v.is_object()) else {return};
    let total=["/input","/output","/reasoning","/cache/read","/cache/write"].iter()
        .filter_map(|path|tokens.pointer(path).and_then(Value::as_u64)).sum::<u64>();
    tokens["total"]=json!(total);
}

fn tool_part(session: &str, message: &str, tool: &Value) -> Value {
    let call = string(tool, "id");
    let mut state = tool.get("state").cloned().unwrap_or_else(|| json!({}));
    if let Some(content) = state.get("content") {
        state["output"] = json!(output(content));
    }
    if let Some(error) = state.get("error").filter(|v| v.is_object()) {
        state["error"] = json!(string(error, "message"));
    }
    if let Some(time) = tool.get("time") {
        state["time"] = json!({"start":time.get("ran").or_else(|| time.get("created")),"end":time.get("completed")});
    }
    json!({"id":tool_id(message, call),"sessionID":session,"messageID":message,
        "type":"tool","callID":call,"tool":string(tool,"name"),"state":state})
}

/// Text ordinals count independently of reasoning ordinals in v2's event stream.
pub fn message(session: &str, native: &Value) -> OpencodeMessage {
    let id = string(native, "id");
    let kind = string(native, "type");
    let mut info = native.clone();
    info["sessionID"] = json!(session);
    info["role"] = json!(if kind == "user" { "user" } else { "assistant" });
    token_total(&mut info);
    if let Some(model) = native.get("model") {
        info["providerID"] = model["providerID"].clone();
        info["modelID"] = model["id"].clone();
    }
    let mut parts = Vec::new();
    match kind {
        "user" => {
            parts.push(json!({"id":part_id(id,"text",0),"type":"text","text":native["text"],"messageID":id,"sessionID":session}));
            for (index, file) in native.get("files").and_then(Value::as_array).into_iter().flatten().enumerate() {
                parts.push(json!({"id":part_id(id,"file",index as u64),"type":"file","mime":file["mime"],
                    "filename":file["name"],"messageID":id,"sessionID":session}));
            }
        }
        "assistant" => {
            let mut ordinals: HashMap<&str, u64> = HashMap::new();
            for content in native.get("content").and_then(Value::as_array).into_iter().flatten() {
                let kind = string(content,"type");
                if kind == "tool" {
                    parts.push(tool_part(session,id,content));
                } else if matches!(kind,"text"|"reasoning") {
                    let ordinal = ordinals.entry(kind).or_default();
                    parts.push(json!({"id":part_id(id,kind,*ordinal),"type":kind,"text":content["text"],
                        "messageID":id,"sessionID":session,"time":{"start":native.pointer("/time/created"),
                        "end":native.pointer("/time/completed")}}));
                    *ordinal += 1;
                }
            }
        }
        "compaction" => {
            if native.get("error").is_none_or(Value::is_null) {
                parts.push(json!({"id":part_id(id,"compaction",0),"type":"compaction","auto":native["reason"]=="auto"}));
            }
        }
        "shell" => {
            parts.push(json!({"id":part_id(id,"shell",0),"type":"tool","tool":"shell","messageID":id,"sessionID":session,
                "state":{"status":if native["status"]=="running"{"running"}else if native["exit"].as_i64().is_some_and(|v|v!=0){"error"}else{"completed"},
                "input":{"command":native["command"]},"output":native.pointer("/output/output").or_else(||native.get("output")).and_then(Value::as_str).unwrap_or(""),
                "error":native.get("error").and_then(|e|e.get("message")).and_then(Value::as_str).unwrap_or("Shell command failed")}}));
        }
        _ => {}
    }
    OpencodeMessage { info, parts }
}

pub fn messages(session: &str, native: &Value) -> Vec<OpencodeMessage> {
    let mut result=message(session,native);
    if native["type"]!="shell" {return vec![result];}
    let id=string(native,"id");
    let user=message(session,&json!({"id":id,"type":"user","text":format!("!{}",string(native,"command")),"time":native["time"]}));
    result.info["id"]=json!(format!("{id}:shell"));
    result.info["parentID"]=json!(id);
    for part in &mut result.parts {part["messageID"]=result.info["id"].clone();}
    vec![user,result]
}

pub fn permission(native: &Value) -> Value {
    let mut value = native.clone();
    value["permission"] = native["action"].clone();
    value["patterns"] = native["resources"].clone();
    value["always"] = native["save"].clone();
    value["tool"] = json!({"callID":native.pointer("/source/id").or_else(||native.pointer("/source/callID"))});
    value
}

/// Keep typed form values and option identifiers; translated labels are never sent as values.
pub fn form(native: &Value) -> Value {
    let mut properties = serde_json::Map::new();
    let mut required = Vec::new();
    let mut order = Vec::new();
    for field in native.get("fields").and_then(Value::as_array).into_iter().flatten() {
        let key = string(field,"key");
        order.push(key.to_string());
        let kind = string(field,"type");
        let mut schema = json!({"type":if kind=="multiselect"{"array"}else if kind=="external"{"boolean"}else{kind},
            "title":field["title"],"description":field["description"]});
        for name in ["default","minLength","maxLength","minimum","maximum","minItems","maxItems","when","url","custom"] {
            if let Some(value) = field.get(name) { schema[name] = value.clone(); }
        }
        if let Some(options) = field.get("options").and_then(Value::as_array) {
            let choices: Vec<Value> = options.iter().map(|o|json!({"const":o["value"],"title":o["label"]})).collect();
            if kind == "multiselect" { schema["items"] = json!({"oneOf":choices}); }
            else { schema["oneOf"] = json!(choices); }
        }
        if field["required"] == true { required.push(key.to_string()); }
        properties.insert(key.to_string(),schema);
    }
    json!({"id":native["id"],"subtype":"elicitation","mode":"form","title":native["title"],
        "display_name":"OpenCode","mcp_server_name":"OpenCode","_opencodeKind":"form",
        "sessionID":native["sessionID"],"_opencodeSessionId":native["sessionID"],"requested_schema":{"type":"object","properties":properties,"required":required,"propertyOrder":order}})
}

#[derive(Default)]
pub struct Projection {
    messages: HashMap<String, Value>,
    tools: HashMap<String, Value>,
    shells: HashMap<String,(String,String)>,
}

fn event(kind: &str, properties: Value) -> Value {
    json!({"type":kind,"properties":properties})
}

impl Projection {
    /// Reload message caches while retaining shell handles owned by an in-flight HTTP request.
    pub fn restore(&mut self, mut history: Self) {
        history.shells=std::mem::take(&mut self.shells);
        *self=history;
    }

    pub fn active_shells(&self, session: &str) -> Vec<String> {
        self.shells.iter().filter(|(_, (owner,_))|owner==session).map(|(id,_)|id.clone()).collect()
    }

    pub fn seed(&mut self, messages: &[OpencodeMessage]) {
        for message in messages {
            let id=string(&message.info,"id");
            if message.info["role"]=="assistant" && message.info.pointer("/time/completed").is_none_or(Value::is_null) {
                self.messages.insert(id.to_string(),message.info.clone());
            }
            for part in &message.parts {
                if part["type"]=="tool" && matches!(part.pointer("/state/status").and_then(Value::as_str),Some("running"|"pending"|"streaming")) {
                    self.tools.insert(string(part,"id").to_string(),part.clone());
                }
            }
        }
    }

    /// Produce the shared event contract without performing IO or holding the engine's timeline lock.
    pub fn events(&mut self, native: &Value) -> Vec<Value> {
        let kind = string(native,"type");
        let data = native.get("data").unwrap_or(&Value::Null);
        let session = string(data,"sessionID");
        let id = string(data,"assistantMessageID");
        let created = native.get("created").cloned().unwrap_or(Value::Null);
        match kind {
            "session.inbox.enqueued" => {
                let item=&data["item"];
                let text=if item["type"]=="user" {item.pointer("/payload/text").and_then(Value::as_str).map(str::to_string)}
                    else if item["type"]=="command" {Some(format!("/{} {}",item.pointer("/payload/command").and_then(Value::as_str).unwrap_or(""),item.pointer("/payload/arguments").and_then(Value::as_str).unwrap_or("")))}
                    else {None};
                let Some(text)=text else {return Vec::new()};
                let message=message(session,&json!({"id":data["inboxID"],"type":"user","text":text,"time":{"created":created}}));
                let mut result=vec![event("message.updated",json!({"info":message.info}))];
                result.extend(message.parts.into_iter().map(|part|event("message.part.updated",json!({"part":part}))));
                result
            }
            "session.shell.started" | "session.shell.ended" => {
                let shell=&data["shell"];
                let shell_id=string(shell,"id");
                let message_id=self.shells.entry(shell_id.to_string()).or_insert_with(||(session.to_string(),string(native,"id").replacen("evt_","msg_",1))).1.clone();
                let value=json!({"id":message_id,"type":"shell","command":shell["command"],"exit":shell["exit"],
                    "status":if kind.ends_with("started"){"running"}else{"exited"},"output":data["output"],
                    "time":{"created":shell.pointer("/time/started"),"completed":if kind.ends_with("ended"){shell.pointer("/time/completed")}else{None}}});
                let mut result=Vec::new();
                for message in messages(session,&value) {
                    result.push(event("message.updated",json!({"info":message.info})));
                    for part in message.parts {result.push(event("message.part.updated",json!({"part":part})));}
                }
                if kind.ends_with("ended") {
                    self.shells.remove(shell_id);
                    result.push(event("session.idle",json!({"sessionID":session})));
                } else {
                    result.insert(0,event("session.status",json!({"sessionID":session,"status":{"type":"busy"}})));
                }
                result
            }
            "session.execution.started" => vec![event("session.status",json!({"sessionID":session,"status":{"type":"busy"}}))],
            "session.execution.succeeded" | "session.execution.interrupted" => vec![event("session.idle",json!({"sessionID":session}))],
            "session.execution.failed" => vec![event("session.error",json!({"sessionID":session,"error":data["error"]})),event("session.idle",json!({"sessionID":session}))],
            "session.renamed" => vec![event("session.updated",json!({"info":{"id":session,"title":data["title"]}}))],
            "session.step.started" => {
                // Completed messages remain in the timeline; only active wire state is cached here.
                let info = json!({"id":id,"sessionID":session,"role":"assistant","agent":data["agent"],
                    "providerID":data.pointer("/model/providerID"),"modelID":data.pointer("/model/id"),"time":{"created":created}});
                self.messages.insert(id.to_string(),info.clone());
                vec![event("message.updated",json!({"info":info}))]
            }
            "session.step.ended" | "session.step.failed" => {
                let mut info = self.messages.remove(id).unwrap_or_else(||json!({"id":id,"sessionID":session,"role":"assistant","time":{}}));
                info["time"]["completed"] = created;
                for key in ["tokens","cost","finish","error"] {
                    if let Some(value)=data.get(key) { info[key]=value.clone(); }
                }
                token_total(&mut info);
                self.tools.retain(|_,part|part["messageID"]!=id);
                vec![event("message.updated",json!({"info":info}))]
            }
            "session.text.started" | "session.reasoning.started" | "session.text.ended" | "session.reasoning.ended" => {
                let text_kind = if kind.contains("reasoning") {"reasoning"}else{"text"};
                let part = json!({"id":part_id(id,text_kind,data["ordinal"].as_u64().unwrap_or(0)),"messageID":id,"sessionID":session,
                    "type":text_kind,"text":data.get("text").and_then(Value::as_str).unwrap_or(""),
                    "time":{"start":created,"end":if kind.ends_with("ended"){native.get("created")}else{None}}});
                vec![event("message.part.updated",json!({"part":part}))]
            }
            "session.text.delta" | "session.reasoning.delta" => {
                let text_kind = if kind.contains("reasoning") {"reasoning"}else{"text"};
                vec![event("message.part.delta",json!({"sessionID":session,"messageID":id,
                    "partID":part_id(id,text_kind,data["ordinal"].as_u64().unwrap_or(0)),"field":"text","delta":data["delta"]}))]
            }
            "session.tool.input.started" => {
                let call = string(data,"id");
                let part = tool_part(session,id,&json!({"id":call,"name":data["name"],"state":{"status":"pending","input":{}}}));
                self.tools.insert(tool_id(id,call),part.clone());
                vec![event("message.part.updated",json!({"part":part}))]
            }
            "session.tool.called" | "session.tool.progress" | "session.tool.success" | "session.tool.failed" => {
                let key = tool_id(id,string(data,"id"));
                let part = self.tools.entry(key.clone()).or_insert_with(||tool_part(session,id,&json!({"id":data["id"],"name":data.get("name").unwrap_or(&json!("tool")),"state":{}})));
                if let Some(input)=data.get("input") { part["state"]["input"]=input.clone(); }
                if let Some(metadata)=data.get("metadata") { part["state"]["metadata"]=metadata.clone(); }
                part["state"]["status"]=json!(match kind {"session.tool.success"=>"completed","session.tool.failed"=>"error",_=>"running"});
                if let Some(content)=data.get("content") { part["state"]["output"]=json!(output(content)); }
                if let Some(error)=data.get("error") { part["state"]["error"]=json!(string(error,"message")); }
                vec![event("message.part.updated",json!({"part":part}))]
            }
            "session.retry.scheduled" => vec![event("session.status",json!({"sessionID":session,"status":{"type":"retry","attempt":data["attempt"],"message":data.pointer("/error/message")}}))],
            "session.compaction.started" => vec![event("message.part.updated",json!({"part":{"id":part_id(session,"compaction",0),"sessionID":session,"type":"compaction","auto":data["reason"]=="auto"}}))],
            "session.compaction.ended" => vec![event("session.compacted",json!({"sessionID":session}))],
            "session.compaction.failed" => vec![event("session.compaction.failed",json!({"sessionID":session})),event("session.error",json!({"sessionID":session,"error":data["error"]}))],
            "permission.asked" => vec![event("permission.asked",permission(data))],
            "permission.replied" | "permission.rejected" => vec![event("permission.replied",data.clone())],
            "form.created" => vec![event("form.created",form(&data["form"]))],
            "form.replied" | "form.cancelled" => vec![event("question.replied",data.clone())],
            _ => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_parts_and_history_have_the_same_stable_identity() {
        let native=json!({"id":"msg_reply","type":"assistant","model":{"id":"model","providerID":"provider"},
            "time":{"created":100,"completed":200},"content":[{"type":"reasoning","text":"think"},
            {"type":"text","text":"first"},{"type":"text","text":"second"},
            {"id":"call_1","type":"tool","name":"shell","state":{"status":"completed","input":{"command":"pwd"},"content":[{"type":"text","text":"/tmp"}]}}]});
        let history=message("ses_main",&native);
        let mut projection=Projection::default();
        let start=projection.events(&json!({"created":100,"type":"session.step.started","data":{"sessionID":"ses_main","assistantMessageID":"msg_reply","model":native["model"]}}));
        assert_eq!(start[0].pointer("/properties/info/modelID"),Some(&json!("model")));
        for (kind,ordinal,text,index) in [("reasoning",0,"think",0),("text",0,"first",1),("text",1,"second",2)] {
            let events=projection.events(&json!({"created":200,"type":format!("session.{kind}.ended"),
                "data":{"sessionID":"ses_main","assistantMessageID":"msg_reply","ordinal":ordinal,"text":text}}));
            assert_eq!(events[0].pointer("/properties/part/id"),history.parts[index].get("id"));
        }
        projection.events(&json!({"type":"session.tool.input.started","data":{"sessionID":"ses_main","assistantMessageID":"msg_reply","id":"call_1","name":"shell"}}));
        projection.events(&json!({"type":"session.tool.called","data":{"sessionID":"ses_main","assistantMessageID":"msg_reply","id":"call_1","input":{"command":"pwd"}}}));
        let events=projection.events(&json!({"type":"session.tool.success","data":{"sessionID":"ses_main","assistantMessageID":"msg_reply","id":"call_1","content":[{"type":"text","text":"/tmp"}]}}));
        assert_eq!(events[0].pointer("/properties/part/id"),history.parts[3].get("id"));
        assert_eq!(events[0].pointer("/properties/part/state/input"),Some(&json!({"command":"pwd"})));
        assert_eq!(events[0].pointer("/properties/part/state/output"),Some(&json!("/tmp")));
    }

    #[test]
    fn permissions_forms_subagents_and_failure_keep_native_fields() {
        let permit=permission(&json!({"id":"per_1","sessionID":"ses_child","action":"shell","resources":["pwd"],"save":["pwd *"],"source":{"id":"call","messageID":"msg"}}));
        assert_eq!(permit.pointer("/tool/callID"),Some(&json!("call")));
        assert_eq!(permit["always"],json!(["pwd *"]));
        let card=form(&json!({"id":"frm_1","sessionID":"ses_child","title":"Choose deployment","fields":[
            {"key":"region","type":"string","required":true,"options":[{"value":"eu","label":"Europe"}]},
            {"key":"tags","type":"multiselect","default":["one"],"options":[{"value":"one","label":"First"}]}]}));
        assert_eq!(card.pointer("/requested_schema/properties/region/oneOf/0/const"),Some(&json!("eu")));
        assert_eq!(card.pointer("/requested_schema/properties/tags/type"),Some(&json!("array")));
        assert_eq!(card["_opencodeSessionId"],json!("ses_child"));
        assert_eq!(card.pointer("/requested_schema/propertyOrder"),Some(&json!(["region","tags"])));
        let native=json!({"id":"msg_parent","type":"assistant","content":[{"id":"call","type":"tool","name":"subagent",
            "state":{"status":"running","input":{"agent":"explore"},"metadata":{"sessionID":"ses_child"}}}]});
        let normalized=message("ses_parent",&native);
        assert_eq!(super::super::opencode_timeline::child_session_of(&normalized.parts[0]).as_deref(),Some("ses_child"));
        let events=Projection::default().events(&json!({"type":"session.execution.failed","data":{"sessionID":"ses_main","error":{"type":"provider.no-route","message":"Model unavailable"}}}));
        assert_eq!(events[0]["type"],"session.error");assert_eq!(events[1]["type"],"session.idle");
        let failed=message("ses_main",&json!({"id":"msg_compact","type":"compaction","error":{"type":"compaction.unavailable","message":"Nothing to compact"}}));
        assert!(failed.parts.is_empty());
    }

    #[test]
    fn shell_replay_preserves_command_result_and_rewind_identity() {
        let rows=messages("ses_main",&json!({"id":"msg_shell","type":"shell","command":"pwd","status":"exited","exit":0,"output":{"output":"/tmp"}}));
        assert_eq!(rows[0].info["id"],"msg_shell");
        assert_eq!(rows[0].parts[0]["text"],"!pwd");
        assert_eq!(rows[1].parts[0].pointer("/state/output"),Some(&json!("/tmp")));
        let mut projection=Projection::default();
        let start=projection.events(&json!({"id":"evt_shell","type":"session.shell.started","data":{"sessionID":"ses_main","shell":{"id":"sh_1","command":"pwd"}}}));
        assert_eq!(start[0].pointer("/properties/status/type"),Some(&json!("busy")));
        assert_eq!(projection.active_shells("ses_main"),vec!["sh_1"]);
        assert!(projection.active_shells("ses_other").is_empty());
        let mut restored=Projection::default();
        restored.seed(&rows);
        projection.restore(restored);
        assert_eq!(projection.active_shells("ses_main"),vec!["sh_1"]);
        let events=projection.events(&json!({"id":"evt_different","type":"session.shell.ended","data":{"sessionID":"ses_main","shell":{"id":"sh_1","command":"pwd","exit":0},"output":{"output":"/tmp"}}}));
        assert_eq!(events[0].pointer("/properties/info/id"),Some(&json!("msg_shell")));
        assert_eq!(events[3].pointer("/properties/part/id"),rows[1].parts[0].get("id"));
        assert_eq!(events.last().unwrap()["type"],"session.idle");
        assert!(projection.active_shells("ses_main").is_empty());
    }
}

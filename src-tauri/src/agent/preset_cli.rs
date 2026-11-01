//! `vpreset`: list the presets VelaTerm follows from `~/.config/velaterm/presets.json`, and link sessions
//! to them or unlink them, so a rollout script can attach existing sessions to their roles in one pass.

use std::io::Read;
use std::time::Instant;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{agent::preset_file, command_core as core, host::AppCtx};

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Request {
    pub session_id: String,
    /// `list`, `link` or `unlink`.
    pub action: String,
    #[serde(default)]
    pub target: Option<String>,
    #[serde(default)]
    pub preset: Option<String>,
}

pub fn run(app: &AppCtx, req: &Request) -> Result<Value, String> {
    super::tell::session(app, &req.session_id)?;
    match req.action.as_str() {
        "list" => {
            preset_file::refresh(app);
            let presets = {
                let conn = app.db().conn.lock().map_err(|_| "The database is unavailable")?;
                preset_file::list(&conn)?
            };
            let file = preset_file::file_path().map(|p| p.display().to_string());
            Ok(json!({"file": file, "presets": presets}))
        }
        "link" | "unlink" => {
            let target = super::tell::resolve(app, req.target.as_deref().ok_or("Specify a session")?)?;
            let session = super::tell::session(app, &target)?;
            let preset = if req.action == "link" {
                Some(req.preset.as_deref().ok_or("Specify a preset id")?)
            } else {
                None
            };
            let changed = core::set_session_preset(app, &target, preset)?;
            let running = app.pty().is_running(&target) || app.chat().is_alive(&target);
            let result = match (&changed, preset) {
                (_, Some(_)) => "linked",
                (Some(_), None) => "unlinked",
                (None, None) => "was not linked",
            };
            Ok(json!({"sessionId": target, "name": session.name, "preset": changed, "result": result, "running": running}))
        }
        other => Err(format!("Unknown action {other}")),
    }
}

pub fn handle(app: AppCtx, mut request: crate::diagnostics::HttpRequest, token: String) {
    let started = Instant::now();
    let request_id = request.id().to_owned();
    let authorized = request
        .headers()
        .iter()
        .any(|h| h.field.equiv("Authorization") && h.value.as_str() == format!("Bearer {token}"));
    let mut body = String::new();
    let result = if !authorized {
        Err((401, "Authentication required".to_owned()))
    } else if request.method() != &tiny_http::Method::Post {
        Err((405, "Use POST".to_owned()))
    } else if request.as_reader().take(65537).read_to_string(&mut body).is_err() || body.len() > 65536 {
        Err((413, "Request too large".to_owned()))
    } else {
        serde_json::from_str::<Request>(&body)
            .map_err(|e| e.to_string())
            .and_then(|req| run(&app, &req))
            .map_err(|e| (400, e))
    };
    let (status, value) = match result {
        Ok(result) => (200, json!({"result":result})),
        Err((status, error)) => (status, json!({"error":error})),
    };
    let _ = request.respond(
        tiny_http::Response::from_string(value.to_string())
            .with_status_code(status)
            .with_header(tiny_http::Header::from_bytes("Content-Type", "application/json").unwrap()),
    );
    crate::diagnostics::record(
        if status == 200 { "INFO" } else { "WARN" },
        "session_preset_request",
        json!({"requestId":request_id,"statusCode":status,"durationMs":started.elapsed().as_millis() as u64}),
    );
}

const USAGE: &str = "usage: vpreset list [--json]
       vpreset link <session> <preset-id> [--json]
       vpreset unlink <session> [--json]
Link sessions to the presets VelaTerm follows from ~/.config/velaterm/presets.json. A linked session
takes its preset's current arguments, permission mode and settings file each time it starts, so an edit
to the file applies at its next resume. Unlinking keeps the values the preset last gave it.
Sessions accept a session ID, an ID prefix of at least 8 characters, or an unambiguous name.
Links and unlinks are written to ~/.config/velaterm/presets-import.log.";

/// Parsed command line: the request and whether to print JSON. `None` asks for the usage text.
fn parse_args(args: &[String]) -> Result<Option<(Request, bool)>, String> {
    let mut json = false;
    let mut words = Vec::new();
    for arg in args {
        match arg.as_str() {
            "--help" | "-h" => return Ok(None),
            "--json" => json = true,
            _ if arg.starts_with('-') => return Err(format!("Unknown option {arg}")),
            _ => words.push(arg.clone()),
        }
    }
    let mut words = words.into_iter();
    let action = words.next().ok_or("Specify list, link or unlink")?;
    let (target, preset) = match action.as_str() {
        "list" => (None, None),
        "link" => {
            let target = words.next().ok_or("Specify a session")?;
            (Some(target), Some(words.next().ok_or("Specify a preset id")?))
        }
        "unlink" => (Some(words.next().ok_or("Specify a session")?), None),
        other => return Err(format!("Unknown action {other}; use list, link or unlink")),
    };
    if words.next().is_some() {
        return Err("Too many arguments".into());
    }
    Ok(Some((Request { session_id: String::new(), action, target, preset }, json)))
}

fn render(action: &str, value: &Value) -> String {
    let text = |v: &Value| v.as_str().unwrap_or("-").to_owned();
    if action == "list" {
        let mut out = format!("presets file: {}", text(&value["file"]));
        let presets = value["presets"].as_array().cloned().unwrap_or_default();
        if presets.is_empty() {
            out.push_str("\n(no presets)");
        }
        for preset in presets {
            let sessions: Vec<String> = preset["sessions"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|s| {
                    let id = text(&s["sessionId"]);
                    format!("{} ({})", text(&s["name"]), &id[..id.len().min(8)])
                })
                .collect();
            let linked = if sessions.is_empty() { "no linked sessions".to_string() } else { sessions.join(", ") };
            out.push_str(&format!("\n{}  {}: {linked}", text(&preset["id"]), text(&preset["name"])));
        }
        return out;
    }
    let mut out = format!("{} ({}): {}", text(&value["name"]), text(&value["sessionId"]), text(&value["result"]));
    if value["preset"].is_string() {
        out.push_str(&format!(" {}", text(&value["preset"])));
    }
    if action == "link" {
        out.push_str(if value["running"] == true {
            "; it is running, so the preset applies when it next starts"
        } else {
            "; the preset applies when it next starts"
        });
    }
    out
}

pub fn run_cli(args: &[String]) -> ! {
    let mut json = false;
    let mut action = String::new();
    let result = (|| -> Result<Value, String> {
        let Some((mut req, as_json)) = parse_args(&args[args.len().min(2)..])? else {
            println!("{USAGE}");
            std::process::exit(0);
        };
        json = as_json;
        action = req.action.clone();
        req.session_id = std::env::var("VLX_SESSION_ID").map_err(|_| "Run inside a VelaTerm session")?;
        super::tell::post_local("preset", &serde_json::to_value(req).unwrap())
    })();
    match result {
        Ok(value) if json => {
            println!("{value}");
            std::process::exit(0);
        }
        Ok(value) => {
            println!("{}", render(&action, &value));
            std::process::exit(0);
        }
        Err(error) => {
            if json {
                println!("{}", json!({"error": error}));
            } else {
                eprintln!("vpreset: {error}");
            }
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| (*v).into()).collect()
    }

    #[test]
    fn parse_reads_each_action() {
        let (req, json) = parse_args(&args(&["list", "--json"])).unwrap().unwrap();
        assert_eq!((req.action.as_str(), req.target, req.preset, json), ("list", None, None, true));
        let (req, _) = parse_args(&args(&["link", "velaterm-lead", "f-co/velaterm-lead"])).unwrap().unwrap();
        assert_eq!((req.target.as_deref(), req.preset.as_deref()), (Some("velaterm-lead"), Some("f-co/velaterm-lead")));
        let (req, _) = parse_args(&args(&["unlink", "12345678"])).unwrap().unwrap();
        assert_eq!((req.action.as_str(), req.target.as_deref(), req.preset), ("unlink", Some("12345678"), None));
        assert!(parse_args(&args(&["--help"])).unwrap().is_none());
        for values in [vec![], vec!["link", "s"], vec!["unlink"], vec!["list", "x"], vec!["drop", "s"], vec!["list", "--all"]] {
            assert!(parse_args(&args(&values)).is_err(), "{values:?}");
        }
    }

    #[test]
    fn render_lists_presets_with_their_sessions() {
        let value = json!({"file": "/h/.config/velaterm/presets.json", "presets": [
            {"id": "f-co/a", "name": "a", "sessions": [{"sessionId": "0a9b7916-59e1", "name": "a"}]},
            {"id": "f-co/b", "name": "b", "sessions": []},
        ]});
        assert_eq!(
            render("list", &value),
            "presets file: /h/.config/velaterm/presets.json\nf-co/a  a: a (0a9b7916)\nf-co/b  b: no linked sessions"
        );
        let linked = json!({"sessionId": "s1", "name": "a", "preset": "f-co/a", "result": "linked", "running": true});
        assert!(render("link", &linked).starts_with("a (s1): linked f-co/a; it is running"));
        let nothing = json!({"sessionId": "s1", "name": "a", "preset": null, "result": "was not linked"});
        assert_eq!(render("unlink", &nothing), "a (s1): was not linked");
    }

    #[test]
    fn request_rejects_unknown_fields() {
        assert!(serde_json::from_value::<Request>(json!({"sessionId":"s","action":"list"})).is_ok());
        assert!(serde_json::from_value::<Request>(json!({"sessionId":"s","action":"list","force":true})).is_err());
    }
}

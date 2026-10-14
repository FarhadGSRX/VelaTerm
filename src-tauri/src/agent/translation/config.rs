//! Backend-owned translation settings, service credentials, and per-conversation preferences.

use super::*;

pub const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS translation_config (id INTEGER PRIMARY KEY CHECK(id=1), value TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS translation_services (id TEXT PRIMARY KEY, name TEXT NOT NULL, base_url TEXT NOT NULL, model TEXT NOT NULL, api_key TEXT NOT NULL DEFAULT '');
CREATE TABLE IF NOT EXISTS translation_preferences (session_id TEXT PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE, value TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS translation_defaults (id INTEGER PRIMARY KEY CHECK(id=1), value TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS translation_jobs (id TEXT PRIMARY KEY, session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE, message_id TEXT NOT NULL, original TEXT NOT NULL, images_hash TEXT NOT NULL, images TEXT NOT NULL, behavior TEXT NOT NULL, source TEXT NOT NULL, target TEXT NOT NULL, version INTEGER NOT NULL, generation INTEGER NOT NULL, state TEXT NOT NULL, result TEXT, error TEXT, attempt INTEGER NOT NULL DEFAULT 0, started_at INTEGER, duration_ms INTEGER, UNIQUE(session_id,message_id));
CREATE TABLE IF NOT EXISTS translation_replies (session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE, row_id TEXT NOT NULL, source_text TEXT NOT NULL, source TEXT NOT NULL, target TEXT NOT NULL, version INTEGER NOT NULL, generation INTEGER NOT NULL, state TEXT NOT NULL, result TEXT, error TEXT, auto INTEGER NOT NULL DEFAULT 0, started_at INTEGER, duration_ms INTEGER, PRIMARY KEY(session_id,row_id));
CREATE TABLE IF NOT EXISTS translation_row_bindings (session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE, row_id TEXT NOT NULL, native_id TEXT NOT NULL, PRIMARY KEY(session_id,row_id), UNIQUE(session_id,native_id));
CREATE TABLE IF NOT EXISTS translation_cache (hash TEXT NOT NULL, source TEXT NOT NULL, target TEXT NOT NULL, version INTEGER NOT NULL, result TEXT NOT NULL, PRIMARY KEY(hash,source,target,version));
";

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Config {
    pub enabled: bool,
    #[serde(default = "default_conversation_kinds")]
    pub conversation_kinds: Vec<SessionKind>,
    pub engine: String,
    pub agent: String,
    pub model: String,
    #[serde(default)]
    pub effort: String,
    pub service_id: String,
    pub version: u64,
    pub generation: u64,
    pub enabled_since: u64,
}
impl Default for Config {
    fn default() -> Self { Self { enabled:false, conversation_kinds:default_conversation_kinds(), engine:"agent".into(), agent:String::new(), model:String::new(), effort:String::new(),
        service_id:String::new(), version:1, generation:1, enabled_since:0 } }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Preferences {
    pub outgoing: bool, pub replies: bool,
    pub source: String, pub target: String,
    pub reply_source: String, pub reply_target: String,
    pub display: String, pub replies_since: u64,
}
impl Default for Preferences {
    fn default() -> Self { Self { outgoing:true, replies:true, source:"auto".into(), target:String::new(),
        reply_source:"auto".into(), reply_target:String::new(), display:"immersive".into(), replies_since:0 } }
}

// Remember language choices without carrying one conversation's switches into another.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LanguageDefaults {
    source: String, target: String, reply_source: String, reply_target: String, display: String,
    #[serde(default)]
    switch_defaults_version: u8,
}
impl From<Preferences> for LanguageDefaults {
    fn from(p: Preferences) -> Self { Self { source:p.source,target:p.target,reply_source:p.reply_source,reply_target:p.reply_target,display:p.display,switch_defaults_version:0 } }
}

pub fn migrate(conn: &rusqlite::Connection) -> Result<(),String> {
    let tx=conn.unchecked_transaction().map_err(db_error)?;
    for table in ["translation_jobs","translation_replies"] {
        let mut stmt=tx.prepare(&format!("PRAGMA table_info({table})")).map_err(db_error)?;
        let columns=stmt.query_map([],|r|r.get::<_,String>(1)).map_err(db_error)?.collect::<Result<Vec<_>,_>>().map_err(db_error)?;
        drop(stmt);
        for column in ["started_at","duration_ms"] {
            if !columns.iter().any(|name|name==column) {tx.execute(&format!("ALTER TABLE {table} ADD COLUMN {column} INTEGER"),[]).map_err(db_error)?;}
        }
    }
    if !tx.query_row("SELECT EXISTS(SELECT 1 FROM translation_defaults WHERE id=1)",[],|r|r.get::<_,bool>(0)).map_err(db_error)? {
        let mut defaults=LanguageDefaults::from(Preferences::default());
        // Legacy rows have no update timestamp. Recover each direction from the newest stored row
        // that contains it; subsequent saves update the singleton transactionally.
        let mut stmt=tx.prepare("SELECT value FROM translation_preferences ORDER BY rowid DESC").map_err(db_error)?;
        let rows=stmt.query_map([],|r|r.get::<_,String>(0)).map_err(db_error)?;
        let mut display_found=false;
        for raw in rows {
            let Ok(p)=serde_json::from_str::<Preferences>(&raw.map_err(db_error)?) else { continue; };
            if defaults.target.is_empty() && !p.target.is_empty() && language(&p.target,false).is_ok() && language(&p.source,true).is_ok() { defaults.source=p.source;defaults.target=p.target; }
            if defaults.reply_target.is_empty() && !p.reply_target.is_empty() && language(&p.reply_target,false).is_ok() && language(&p.reply_source,true).is_ok() { defaults.reply_source=p.reply_source;defaults.reply_target=p.reply_target; }
            if !display_found && ["original","translation","both","immersive"].contains(&p.display.as_str()) { defaults.display=p.display;display_found=true; }
        }
        drop(stmt);
        tx.execute("INSERT INTO translation_defaults(id,value) VALUES(1,?1)",[serde_json::to_string(&defaults).unwrap()]).map_err(db_error)?;
    }
    let mut defaults=read_defaults(&tx)?;
    if defaults.switch_defaults_version==0 {
        enable_conversation_defaults(&tx)?;
        defaults.switch_defaults_version=1;
        tx.execute("UPDATE translation_defaults SET value=?1 WHERE id=1",[serde_json::to_string(&defaults).unwrap()]).map_err(db_error)?;
    }
    tx.commit().map_err(db_error)
}

// A global enable starts every conversation enabled; later local pauses remain independent.
fn enable_conversation_defaults(conn: &rusqlite::Connection) -> Result<(),String> {
    let mut stmt=conn.prepare("SELECT session_id,value FROM translation_preferences").map_err(db_error)?;
    let rows=stmt.query_map([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?))).map_err(db_error)?.collect::<Result<Vec<_>,_>>().map_err(db_error)?;
    drop(stmt);
    for (id,raw) in rows {
        let mut prefs:Preferences=serde_json::from_str(&raw).map_err(|_|"translation:configuration")?;
        if prefs.outgoing {continue;}
        prefs.outgoing=true;
        conn.execute("UPDATE translation_preferences SET value=?1 WHERE session_id=?2",params![serde_json::to_string(&prefs).unwrap(),id]).map_err(db_error)?;
    }
    Ok(())
}

fn read_defaults(conn: &rusqlite::Connection) -> Result<LanguageDefaults,String> {
    let raw:String=conn.query_row("SELECT value FROM translation_defaults WHERE id=1",[],|r|r.get(0)).map_err(db_error)?;
    serde_json::from_str(&raw).map_err(|_|"translation:configuration".into())
}

// Names are native language labels, independent of the user's interface locale.
pub const LANGUAGES: &[(&str, &str)] = &[
    ("en","English"),("zh-CN","简体中文"),("zh-TW","繁體中文"),("ja","日本語"),("ko","한국어"),
    ("de","Deutsch"),("es","Español"),("fr","Français"),("it","Italiano"),("pt-BR","Português (Brasil)"),
    ("ru","Русский"),("vi","Tiếng Việt"),("ar","العربية"),("hi","हिन्दी"),("id","Bahasa Indonesia"),
    ("nl","Nederlands"),("pl","Polski"),("tr","Türkçe"),("uk","Українська"),("th","ไทย"),
];
pub const AGENTS: &[SessionKind] = &[SessionKind::Claude,SessionKind::Codex,SessionKind::Opencode,SessionKind::Pi,SessionKind::Omp,SessionKind::Grok];

fn default_conversation_kinds() -> Vec<SessionKind> {
    super::super::launch_options::catalog().into_iter().filter(|s|s.id.supports_chat()).map(|s|s.id).collect()
}

pub fn enabled_for(conn: &rusqlite::Connection, cfg: &Config, id: &str) -> Result<bool,String> {
    if !cfg.enabled {return Ok(false);}
    let session=repo::get_session(conn,id)?.ok_or("translation:session")?;
    Ok(session.engine=="chat" && session.archived_at.is_none() && cfg.conversation_kinds.contains(&session.kind))
}

pub fn set_conversation_kinds(app: &AppCtx, value: &Value) -> Result<(),String> {
    let requested:Vec<SessionKind>=serde_json::from_value(value.clone()).map_err(|_|"translation:configuration")?;
    let supported=default_conversation_kinds();
    if requested.iter().any(|kind|!supported.contains(kind)) {return Err("translation:configuration".into());}
    let kinds:Vec<_>=supported.into_iter().filter(|kind|requested.contains(kind)).collect();
    let _gate=GATE.lock().unwrap();let conn=app.db().conn.lock().unwrap();let mut cfg=read_config(&conn)?;
    if cfg.conversation_kinds==kinds {return Ok(());}
    cfg.conversation_kinds=kinds;cfg.generation+=1;
    let tx=conn.unchecked_transaction().map_err(db_error)?;
    tx.execute("INSERT INTO translation_config(id,value) VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET value=excluded.value",[serde_json::to_string(&cfg).unwrap()]).map_err(db_error)?;
    // Changing the scope invalidates pending delivery; canceled drafts remain recoverable.
    cancel_pending(&tx)?;
    tx.commit().map_err(db_error)?;
    drop(conn);app.emit("translation://settings",json!({}));Ok(())
}

fn cancel_pending(conn: &rusqlite::Connection) -> Result<(),String> {
    conn.execute("UPDATE translation_jobs SET state='canceled',duration_ms=CASE WHEN started_at IS NULL THEN duration_ms ELSE COALESCE(duration_ms,0)+MAX(0,?1-started_at) END,started_at=NULL,error='translation:canceled' WHERE state IN ('pending','ready','failed')",[now_ms()]).map_err(db_error)?;
    conn.execute("UPDATE translation_replies SET state='canceled',duration_ms=CASE WHEN started_at IS NULL THEN duration_ms ELSE COALESCE(duration_ms,0)+MAX(0,?1-started_at) END,started_at=NULL WHERE state='pending'",[now_ms()]).map_err(db_error)?;
    Ok(())
}

pub fn language(value: &str, source: bool) -> Result<(), String> {
    if source && value == "auto" || LANGUAGES.iter().any(|(code,_)| *code == value) { Ok(()) }
    else { Err("translation:language".into()) }
}

pub fn read_config(conn: &rusqlite::Connection) -> Result<Config, String> {
    let raw: Option<String> = conn.query_row("SELECT value FROM translation_config WHERE id=1", [], |r|r.get(0)).optional().map_err(db_error)?;
    raw.map(|s|serde_json::from_str(&s).map_err(|_|"translation:configuration".into())).unwrap_or_else(||Ok(Config::default()))
}
pub fn read_preferences(conn: &rusqlite::Connection, id: &str) -> Result<Preferences, String> {
    let raw: Option<String> = conn.query_row("SELECT value FROM translation_preferences WHERE session_id=?1", [id], |r|r.get(0)).optional().map_err(db_error)?;
    let defaults=read_defaults(conn)?;
    if let Some(raw)=raw {
        let mut prefs:Preferences=serde_json::from_str(&raw).map_err(|_|"translation:configuration")?;
        // Empty legacy directions were never configured. Preserve all explicit choices.
        if prefs.target.is_empty() { prefs.target=defaults.target;if prefs.source=="auto" {prefs.source=defaults.source;} }
        if prefs.reply_target.is_empty() { prefs.reply_target=defaults.reply_target;if prefs.reply_source=="auto" {prefs.reply_source=defaults.reply_source;} }
        Ok(prefs)
    } else {
        Ok(Preferences { source:defaults.source,target:defaults.target,reply_source:defaults.reply_source,
            reply_target:defaults.reply_target,display:defaults.display,..Preferences::default() })
    }
}

pub fn settings(app: &AppCtx, editable: bool) -> Result<Value, String> {
    let conn = app.db().conn.lock().unwrap();
    let config = read_config(&conn)?;
    let mut stmt = conn.prepare("SELECT id,name,base_url,model,api_key!='' FROM translation_services ORDER BY name,id").map_err(db_error)?;
    let services = stmt.query_map([],|r|Ok(json!({"id":r.get::<_,String>(0)?,"name":r.get::<_,String>(1)?,"baseUrl":r.get::<_,String>(2)?,"model":r.get::<_,String>(3)?,"hasKey":r.get::<_,bool>(4)?})))
        .map_err(db_error)?.collect::<Result<Vec<_>,_>>().map_err(db_error)?;
    drop(stmt); drop(conn);
    let agents: Vec<_> = super::super::launch_options::catalog().into_iter().filter(|s|AGENTS.contains(&s.id)).map(|s| {
        let available = super::super::session_title::available_binary(app,s.id,None).is_some();
        let mut entry=serde_json::to_value(s).unwrap();entry["available"]=json!(available);entry
    }).collect();
    Ok(json!({"config":config,"services":services,"agents":agents,"editable":editable,
        "conversationKinds":super::super::launch_options::catalog().into_iter().filter(|s|s.id.supports_chat()).map(|s|json!({"id":s.id,"label":s.label})).collect::<Vec<_>>(),
        "languages":LANGUAGES.iter().map(|(code,name)|json!({"code":code,"name":name})).collect::<Vec<_>>(),
        "engines":["agent","api","google"],"protocols":["openai"],"displays":["original","translation","both","immersive"]}))
}

pub fn enable(app: &AppCtx, enabled: bool) -> Result<(), String> {
    let _gate = GATE.lock().unwrap();
    let conn = app.db().conn.lock().unwrap(); let mut cfg = read_config(&conn)?;
    if cfg.enabled == enabled { return Ok(()); }
    let tx=conn.unchecked_transaction().map_err(db_error)?;
    cfg.enabled = enabled; cfg.generation += 1; cfg.enabled_since = now_ms();
    tx.execute("INSERT INTO translation_config(id,value) VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET value=excluded.value", [serde_json::to_string(&cfg).unwrap()]).map_err(db_error)?;
    if enabled {enable_conversation_defaults(&tx)?;}
    cancel_pending(&tx)?;
    tx.commit().map_err(db_error)?;
    drop(conn); app.emit("translation://settings",json!({})); Ok(())
}

pub fn save(app: &AppCtx, value: &Value) -> Result<(), String> {
    let engine = value["engine"].as_str().unwrap_or_default();
    let agent = value["agent"].as_str().unwrap_or_default().trim();
    let model = value["model"].as_str().unwrap_or_default().trim();
    let effort = value["effort"].as_str().unwrap_or_default().trim();
    let service = value["serviceId"].as_str().unwrap_or_default();
    if !["agent","api","google"].contains(&engine) || model.len()>256 || effort.len()>256 { return Err("translation:configuration".into()); }
    if !agent.is_empty() && !AGENTS.iter().any(|s|s.as_str()==agent) { return Err("translation:agent".into()); }
    if !agent.is_empty() {
        super::super::launch_options::apply(SessionKind::from_db(agent),None,Some(model),Some(effort)).map_err(|_|"translation:configuration")?;
    } else if !effort.is_empty() { return Err("translation:configuration".into()); }
    let _gate = GATE.lock().unwrap(); let conn = app.db().conn.lock().unwrap(); let mut cfg = read_config(&conn)?;
    if !service.is_empty() && !conn.query_row("SELECT EXISTS(SELECT 1 FROM translation_services WHERE id=?1)",[service],|r|r.get::<_,bool>(0)).map_err(db_error)? { return Err("translation:configuration".into()); }
    cfg.engine=engine.into();cfg.agent=agent.into();cfg.model=model.into();cfg.effort=effort.into();cfg.service_id=service.into();cfg.version+=1;cfg.generation+=1;
    conn.execute("INSERT INTO translation_config(id,value) VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET value=excluded.value",[serde_json::to_string(&cfg).unwrap()]).map_err(db_error)?;
    drop(conn); app.emit("translation://settings",json!({})); Ok(())
}

pub fn base_url(raw: &str) -> Result<String,String> {
    let url = url::Url::parse(raw.trim()).map_err(|_|"translation:url")?;
    if !["http","https"].contains(&url.scheme()) || url.host_str().is_none() || !url.username().is_empty()
        || url.password().is_some() || url.query().is_some() || url.fragment().is_some() { return Err("translation:url".into()); }
    Ok(url.as_str().trim_end_matches('/').to_string())
}

pub fn service_save(app: &AppCtx, value: &Value) -> Result<String,String> {
    let name = value["name"].as_str().unwrap_or_default().trim();
    let model = value["model"].as_str().unwrap_or_default().trim();
    let base = base_url(value["baseUrl"].as_str().unwrap_or_default())?;
    if name.is_empty() || name.len()>128 || model.is_empty() || model.len()>256 { return Err("translation:configuration".into()); }
    let id = value["id"].as_str().filter(|s|!s.is_empty()).map(str::to_owned).unwrap_or_else(||uuid::Uuid::new_v4().to_string());
    let _gate = GATE.lock().unwrap(); let conn = app.db().conn.lock().unwrap();
    let existing: Option<(String,String)> = conn.query_row("SELECT base_url,api_key FROM translation_services WHERE id=?1",[&id],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(db_error)?;
    let key = if value["clearKey"] == true { String::new() } else {
        value["apiKey"].as_str().filter(|s|!s.is_empty()).map(str::to_owned)
            .or_else(||existing.as_ref().filter(|(url,_)|url==&base).map(|(_,key)|key.clone())).unwrap_or_default()
    };
    if key.contains(['\n','\r']) || key.len()>8192 { return Err("translation:configuration".into()); }
    conn.execute("INSERT INTO translation_services(id,name,base_url,model,api_key) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(id) DO UPDATE SET name=excluded.name,base_url=excluded.base_url,model=excluded.model,api_key=excluded.api_key",params![id,name,base,model,key]).map_err(db_error)?;
    let mut cfg=read_config(&conn)?;cfg.version+=1;cfg.generation+=1;
    conn.execute("INSERT INTO translation_config(id,value) VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET value=excluded.value",[serde_json::to_string(&cfg).unwrap()]).map_err(db_error)?;
    drop(conn);app.emit("translation://settings",json!({}));Ok(id)
}

pub fn session_save(app: &AppCtx, id: &str, value: &Value) -> Result<Preferences,String> {
    let _gate=GATE.lock().unwrap();let conn=app.db().conn.lock().unwrap();session(&conn,id)?;
    if !enabled_for(&conn,&read_config(&conn)?,id)? { return Err("translation:disabled".into()); }
    let mut prefs: Preferences=serde_json::from_value(value.clone()).map_err(|_|"translation:configuration")?;
    language(&prefs.source,true)?;language(&prefs.reply_source,true)?;
    if !prefs.target.is_empty() { language(&prefs.target,false)?; }
    if !prefs.reply_target.is_empty() { language(&prefs.reply_target,false)?; }
    if !["original","translation","both","immersive"].contains(&prefs.display.as_str()) { return Err("translation:language".into()); }
    let tx=conn.unchecked_transaction().map_err(db_error)?;
    let previous=read_preferences(&conn,id)?;
    let mut defaults=read_defaults(&tx)?;
    // Switching an older conversation on/off must not replace the last chosen languages.
    let changed=previous.source!=prefs.source || previous.target!=prefs.target || previous.reply_source!=prefs.reply_source
        || previous.reply_target!=prefs.reply_target || previous.display!=prefs.display;
    if previous.source!=prefs.source || previous.target!=prefs.target { defaults.source=prefs.source.clone();defaults.target=prefs.target.clone(); }
    if previous.reply_source!=prefs.reply_source || previous.reply_target!=prefs.reply_target { defaults.reply_source=prefs.reply_source.clone();defaults.reply_target=prefs.reply_target.clone(); }
    if previous.display!=prefs.display { defaults.display=prefs.display.clone(); }
    prefs.replies_since = if prefs.replies && (!previous.replies || previous.reply_source!=prefs.reply_source || previous.reply_target!=prefs.reply_target) { now_ms() } else { previous.replies_since };
    if previous.outgoing!=prefs.outgoing || previous.source!=prefs.source || previous.target!=prefs.target {
        tx.execute("UPDATE translation_jobs SET state='canceled',result=NULL,duration_ms=CASE WHEN started_at IS NULL THEN duration_ms ELSE COALESCE(duration_ms,0)+MAX(0,?2-started_at) END,started_at=NULL,error='translation:canceled' WHERE session_id=?1 AND state IN ('pending','ready','failed')",params![id,now_ms()]).map_err(db_error)?;
    }
    tx.execute("INSERT INTO translation_preferences(session_id,value) VALUES(?1,?2) ON CONFLICT(session_id) DO UPDATE SET value=excluded.value",params![id,serde_json::to_string(&prefs).unwrap()]).map_err(db_error)?;
    if changed { tx.execute("UPDATE translation_defaults SET value=?1 WHERE id=1",[serde_json::to_string(&defaults).unwrap()]).map_err(db_error)?; }
    tx.commit().map_err(db_error)?;
    drop(conn);app.emit(&format!("translation://session/{id}"),json!({}));
    if changed { app.emit("translation://defaults",json!({})); }
    Ok(prefs)
}

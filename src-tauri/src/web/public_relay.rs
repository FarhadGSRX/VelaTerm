//! Public account sharing: device linking, share records, and the tunnel that serves shared apps over URLs.

use super::{e2ee::ServerKeys, share_policy::ShareScope};
use crate::host::AppCtx;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{Mutex, OnceLock},
    time::Duration,
};

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LocalShare {
    #[serde(flatten)]
    scope: ShareScope,
    secret: String,
    password_hash: Option<String>,
    #[serde(default)]
    allowed_accounts: Vec<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Config {
    origin: String,
    device_id: String,
    token: String,
    shares: HashMap<String, LocalShare>,
}
struct Linking {
    origin: String,
    code: String,
    poll_token: String,
}
static LINKING: OnceLock<Mutex<HashMap<PathBuf, Linking>>> = OnceLock::new();
static RUNNING: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();
static FILE_LOCK: Mutex<()> = Mutex::new(());
const CONFIG: &str = "vlx-public-sharing.json";

fn random() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}
fn load(app: &AppCtx) -> Result<Config, String> {
    let bytes = std::fs::read(app.data_dir()?.join(CONFIG))
        .map_err(|_| "Link this device to your account first")?;
    serde_json::from_slice(&bytes).map_err(|_| "Invalid public sharing configuration".into())
}
fn save(app: &AppCtx, config: &Config) -> Result<(), String> {
    let path = app.data_dir()?.join(CONFIG);
    let pending = path.with_extension("new");
    super::write_owner_only(
        &pending,
        &serde_json::to_vec(config).map_err(|_| "Cannot encode sharing configuration")?,
    )
    .map_err(|_| "Cannot save sharing configuration")?;
    std::fs::rename(pending, path).map_err(|_| "Cannot save sharing configuration".into())
}
fn request(
    origin: &str,
    path: &str,
    token: Option<&str>,
    body: Option<&Value>,
) -> Result<Value, String> {
    request_method(
        origin,
        path,
        token,
        body,
        if body.is_some() { "POST" } else { "GET" },
    )
}
fn request_method(
    origin: &str,
    path: &str,
    token: Option<&str>,
    body: Option<&Value>,
    method: &str,
) -> Result<Value, String> {
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(40))
        .redirects(0)
        .build();
    let mut req = agent
        .request(method, &format!("{origin}{path}"))
        .set("Origin", origin);
    if let Some(token) = token {
        req = req.set("Authorization", &format!("Bearer {token}"));
    }
    let response = if let Some(body) = body {
        req.set("Content-Type", "application/json")
            .send_string(&body.to_string())
    } else {
        req.call()
    }
    .map_err(|_| "Public account service is unavailable or rejected the request")?;
    use std::io::Read;
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take(3 * 1024 * 1024)
        .read_to_end(&mut bytes)
        .map_err(|_| "Cannot read relay response")?;
    if bytes.is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_slice(&bytes).map_err(|_| "Invalid public account service response".into())
}

/// Device token of the current account link. The tunnel uses it to notice that the account was unlinked or
/// relinked and stop dialing with stale credentials.
pub fn config_token(app: &AppCtx) -> Option<String> {
    load(app).ok().map(|config| config.token)
}

/// Resolves a tunnel-injected grant and account against the host's local share records. Returns the scope
/// only when the host still has that share and the account is allowed to use it.
pub fn share_scope_for(app: &AppCtx, share_id: &str, account_id: &str) -> Option<ShareScope> {
    let config = load(app).ok()?;
    let share = config.shares.get(share_id)?;
    // An empty allow-list means the share was created for the owning account only and no grant sync has
    // filled the owner in yet; treat it as not yet authorized rather than open to everyone.
    if !share
        .allowed_accounts
        .iter()
        .any(|allowed| allowed == account_id)
    {
        return None;
    }
    let mut scope = share.scope.clone();
    scope.push_authority = Some((share_id.to_string(), account_id.to_string()));
    Some(scope)
}

/// Mints a one-time browser session on the relay and returns the URL a desktop window should load. The
/// destination is the selected device's remote page, or the specific grant when one is given.
pub fn browser_ticket_url(
    app: &AppCtx,
    device_id: &str,
    grant_id: Option<&str>,
) -> Result<String, String> {
    let config = load(app)?;
    let mut body = json!({"deviceId": device_id});
    if let Some(grant) = grant_id {
        body["grantId"] = json!(grant);
    }
    let result = request(
        &config.origin,
        "/api/device-link/host/browser-ticket",
        Some(&config.token),
        Some(&body),
    )?;
    result["url"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| "Invalid browser ticket response".into())
}

pub fn dispatch(app: &AppCtx, cmd: &str, args: &Value) -> Result<Value, String> {
    match cmd {
        "public_account_status" => {
            if let Ok(config) = load(app) {
                let status = request(
                    &config.origin,
                    "/api/device-link/host/status",
                    Some(&config.token),
                    None,
                );
                let status = status?;
                if status["linked"] == false {
                    return Ok(json!({"linked":false,"origin":config.origin}));
                }
                start(app)?;
                Ok(
                    json!({"linked":true,"deviceId":config.device_id,"origin":config.origin,"account":status["account"]}),
                )
            } else {
                Ok(json!({"linked":false,"origin":"https://velaterm.com"}))
            }
        }
        "public_account_logout" => {
            let config = load(app)?;
            let status = request(
                &config.origin,
                "/api/device-link/host/status",
                Some(&config.token),
                None,
            )?;
            if status["linked"] == true {
                request(
                    &config.origin,
                    "/api/device-link/host/logout",
                    Some(&config.token),
                    Some(&json!({})),
                )?;
            }
            let _lock = FILE_LOCK
                .lock()
                .map_err(|_| "Sharing configuration unavailable")?;
            std::fs::remove_file(app.data_dir()?.join(CONFIG))
                .map_err(|_| "Cannot remove device credential")?;
            // The tunnel notices the missing configuration and stops; the loopback share server is closed here
            // so a logged-out device keeps no share surface listening.
            super::share_server::stop();
            Ok(Value::Null)
        }
        "public_account_remote_devices" => {
            let config = load(app)?;
            request(
                &config.origin,
                "/api/device-link/host/remote",
                Some(&config.token),
                None,
            )
        }
        "public_account_remote_disable" => {
            let _lock = FILE_LOCK.lock().map_err(|_| "Remote access unavailable")?;
            let mut config = load(app)?;
            let id = args["id"]
                .as_str()
                .filter(|v| uuid::Uuid::parse_str(v).is_ok())
                .ok_or("Invalid remote access ID")?;
            request_method(
                &config.origin,
                &format!("/api/device-link/host/remote/{id}"),
                Some(&config.token),
                None,
                "DELETE",
            )?;
            config.shares.remove(id);
            save(app, &config)?;
            Ok(Value::Null)
        }
        "public_account_link" => {
            let origin = "https://velaterm.com".to_string();
            let keys = ServerKeys::load_or_create(&app.data_dir()?)?;
            let name = sysinfo::System::host_name().unwrap_or_else(|| "VelaTerm".into());
            let result = request(
                &origin,
                "/api/device-link",
                None,
                Some(&json!({"name":name,"publicKey":keys.public_key_b64()})),
            )?;
            let get = |key: &str| {
                result
                    .get(key)
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .ok_or("Invalid device linking response")
            };
            let url = get("url")?;
            LINKING
                .get_or_init(Default::default)
                .lock()
                .map_err(|_| "Device linking unavailable")?
                .insert(
                    app.data_dir()?,
                    Linking {
                        origin,
                        code: get("code")?,
                        poll_token: get("pollToken")?,
                    },
                );
            Ok(json!({"url":url,"publicKey":keys.public_key_b64()}))
        }
        "public_account_poll" => {
            let mut attempts = LINKING
                .get_or_init(Default::default)
                .lock()
                .map_err(|_| "Device linking unavailable")?;
            let dir = app.data_dir()?;
            let attempt = attempts.get(&dir).ok_or("Start account linking first")?;
            let result = request(
                &attempt.origin,
                &format!("/api/device-link/{}/poll", attempt.code),
                Some(&attempt.poll_token),
                Some(&json!({})),
            )?;
            if result.is_null() {
                return Ok(json!({"linked":false}));
            }
            let config = Config {
                origin: attempt.origin.clone(),
                device_id: result["device"]["id"]
                    .as_str()
                    .ok_or("Invalid device ID")?
                    .into(),
                token: result["token"]
                    .as_str()
                    .ok_or("Invalid device credential")?
                    .into(),
                shares: HashMap::new(),
            };
            let _lock = FILE_LOCK
                .lock()
                .map_err(|_| "Sharing configuration unavailable")?;
            save(app, &config)?;
            attempts.remove(&dir);
            start(app)?;
            Ok(json!({"linked":true}))
        }
        "public_account_remote_enable" => {
            let _lock = FILE_LOCK
                .lock()
                .map_err(|_| "Sharing configuration unavailable")?;
            let mut config = load(app)?;
            let scope: ShareScope =
                serde_json::from_value(args.clone()).map_err(|_| "Invalid sharing scope")?;
            // Validate the target against host-owned records before asking the relay to create the grant.
            let tree = crate::command_core::list_tree(app)?;
            let valid = match scope.scope.as_str() {
                "machine" => scope.target_id.is_none(),
                "project" => tree
                    .projects
                    .iter()
                    .any(|p| Some(p.id.as_str()) == scope.target_id.as_deref()),
                "session" => !scope
                    .dispatch(app, "shared_sessions", &json!({}))?
                    .as_array()
                    .ok_or("Invalid sessions")?
                    .is_empty(),
                _ => false,
            };
            if !valid {
                return Err("Invalid remote access target".into());
            }
            let owner = request(
                &config.origin,
                "/api/device-link/host/status",
                Some(&config.token),
                None,
            )?;
            let owner_id = owner["account"]["id"]
                .as_str()
                .ok_or("Account unavailable")?
                .to_string();
            let result = request(
                &config.origin,
                "/api/device-link/host/remote",
                Some(&config.token),
                Some(&json!({"scope":scope.scope,"targetId":scope.target_id})),
            )?;
            let id = result["id"].as_str().ok_or("Invalid share response")?;
            config.shares.insert(
                id.into(),
                LocalShare {
                    scope,
                    secret: random(),
                    password_hash: None,
                    allowed_accounts: vec![owner_id],
                },
            );
            save(app, &config)?;
            let name = scope_name(app, &config.shares[id].scope)?;
            request(
                &config.origin,
                "/api/device-link/host/share-ready",
                Some(&config.token),
                Some(&json!({"id":id,"name":name})),
            )?;
            start(app)?;
            Ok(json!({"id":id}))
        }
        "public_remote_options" => {
            // Scope picker data for the account Remote panel: projects and AI conversations the host could
            // expose. Only names and opaque IDs leave this call.
            let tree = crate::command_core::list_tree(app)?;
            let sessions = ShareScope {
                scope: "machine".into(),
                target_id: None,
                push_authority: None,
            }
            .dispatch(app, "shared_sessions", &json!({}))?;
            Ok(json!({
                "options": {"scopes": ["machine", "project", "session"]},
                "projects": tree.projects.iter().map(|p| json!({"id":p.id,"name":p.name})).collect::<Vec<_>>(),
                "sessions": sessions,
            }))
        }
        "public_account_remote_url" => {
            // Returns the relay URL for the account Remote window. The browser-ticket endpoint mints a
            // one-time browser session with this device's credential, so the window starts logged in.
            let device_id = args["deviceId"]
                .as_str()
                .filter(|v| uuid::Uuid::parse_str(v).is_ok())
                .ok_or("Invalid device ID")?;
            let grant_id = args["grantId"].as_str();
            let url = browser_ticket_url(app, device_id, grant_id)?;
            Ok(json!({"url": url}))
        }
        _ => Err("Unknown public sharing command".into()),
    }
}

fn scope_name(app: &AppCtx, scope: &ShareScope) -> Result<String, String> {
    if scope.scope == "machine" {
        return Ok("Workspace".into());
    }
    let tree = crate::command_core::list_tree(app)?;
    if scope.scope == "project" {
        return Ok(tree
            .projects
            .iter()
            .find(|p| Some(p.id.as_str()) == scope.target_id.as_deref())
            .map(|p| p.name.clone())
            .ok_or_else(|| "Shared project no longer exists".to_string())?);
    }
    let sessions = scope.dispatch(app, "shared_sessions", &json!({}))?;
    sessions[0]["name"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| "Shared conversation no longer exists".to_string())
}

pub(crate) fn sync(app: &AppCtx) -> Result<(), String> {
    sync_grants(app, &load(app)?)
}

fn sync_grants(app: &AppCtx, config: &Config) -> Result<(), String> {
    let grants = request(
        &config.origin,
        "/api/device-link/host/grants",
        Some(&config.token),
        None,
    )?;
    let grants = grants.as_array().ok_or("Invalid grants")?;
    let grant_ids: HashSet<&str> = grants.iter()
        .filter_map(|grant| grant["share"]["id"].as_str()).collect();
    let mut confirmed = Vec::new();
    {
        let _lock = FILE_LOCK.lock().map_err(|_| "Sharing unavailable")?;
        let mut current = load(app)?;
        if current.device_id != config.device_id || current.token != config.token {
            return Err("Account changed".into());
        }
        current.shares.retain(|id, _| grant_ids.contains(id.as_str()));
        for grant in grants {
            let id = grant["share"]["id"].as_str().ok_or("Invalid share")?;
            let Some(local) = current.shares.get_mut(id) else { continue };
            let mut allowed = vec![grant["ownerId"].as_str().ok_or("Invalid owner")?.to_string()];
            if grant["share"]["accessMode"] == "accounts" {
                for account in grant["accountIds"].as_array().ok_or("Invalid accounts")? {
                    allowed.push(account.as_str().ok_or("Invalid account")?.to_string());
                }
            }
            local.allowed_accounts = allowed;
            // A deleted project or conversation is no longer available, even if its grant still exists.
            if let Ok(name) = scope_name(app, &local.scope) {
                confirmed.push(json!({"id":id,"name":name}));
            }
        }
        save(app, &current)?;
    }
    request_method(
        &config.origin,
        "/api/device-link/host/shared-scopes",
        Some(&config.token),
        Some(&json!(confirmed)),
        "PUT",
    )?;
    Ok(())
}


/// Starts serving public share traffic: the loopback share server plus the outbound tunnel, with a grant
/// sync thread keeping local allow-lists in step with the relay. Idempotent per data directory.
pub fn start(app: &AppCtx) -> Result<(), String> {
    let config = load(app)?;
    let dir = app.data_dir()?;
    let mut running = RUNNING
        .get_or_init(Default::default)
        .lock()
        .map_err(|_| "Relay unavailable")?;
    if !running.insert(dir.clone()) {
        return Ok(());
    }
    let (port, secret) = match super::share_server::ensure(app) {
        Ok(value) => value,
        Err(e) => {
            running.remove(&dir);
            return Err(e);
        }
    };
    let token = config.token.clone();
    // Grant sync thread: pulls the relay's authorization records so local account allow-lists stay current.
    {
        let app = app.clone();
        let dir = dir.clone();
        let token = token.clone();
        std::thread::Builder::new()
            .name("share-grants".into())
            .spawn(move || {
                let Ok(config) = load(&app) else { return };
                let _ = sync_grants(&app, &config);
                loop {
                    std::thread::sleep(Duration::from_secs(60));
                    let Ok(config) = load(&app) else { break };
                    if config.token != token {
                        break;
                    }
                    let _ = sync_grants(&app, &config);
                }
                if let Ok(mut running) = RUNNING.get_or_init(Default::default).lock() {
                    running.remove(&dir);
                }
            })
            .map_err(|_| "Cannot start public share grant sync")?;
    }
    super::share_tunnel::start(app.clone(), config.origin, token, port, secret);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::HeadlessHost;

    /// The tunnel-injected grant and account are the only admission a share WebSocket gets; the local
    /// allow-list must reject both unknown grants and accounts that were not authorized.
    #[test]
    fn scope_lookup_honors_local_share_and_accounts() {
        let dir = std::env::temp_dir().join(format!("vlx-share-api-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = crate::db::Db::open(&dir.join("test.db")).unwrap();
        let config = json!({
            "origin": "https://velaterm.com",
            "deviceId": "11111111-1111-1111-1111-111111111111",
            "token": "device-token",
            "shares": {
                "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa": {
                    "scope": "machine",
                    "secret": "s",
                    "allowedAccounts": ["acct-1"]
                }
            }
        });
        std::fs::write(dir.join(CONFIG), config.to_string()).unwrap();
        let app = AppCtx::Headless(std::sync::Arc::new(HeadlessHost::new(dir.clone(), db)));
        assert_eq!(config_token(&app).as_deref(), Some("device-token"));
        assert!(share_scope_for(&app, "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa", "acct-1").is_some());
        assert!(share_scope_for(&app, "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa", "acct-2").is_none());
        assert!(share_scope_for(&app, "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb", "acct-1").is_none());
        drop(app);
        std::fs::remove_dir_all(dir).unwrap();
    }
}

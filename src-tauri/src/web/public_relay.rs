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

/// Account service origin of the current link, when this device is linked.
pub fn account_origin(app: &AppCtx) -> Option<String> {
    load(app).ok().map(|config| config.origin)
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

/// Whether a share connection belongs to the account that owns this host. The owner is always the first
/// allowed account: `enable` records it alone and `sync_grants` puts the grant owner ahead of invitees.
pub fn visitor_is_owner(app: &AppCtx, scope: &ShareScope) -> bool {
    let Some((share_id, account_id)) = &scope.push_authority else {
        return false;
    };
    load(app)
        .ok()
        .and_then(|config| config.shares.get(share_id)?.allowed_accounts.first().cloned())
        .is_some_and(|owner| &owner == account_id)
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
            unlink(app)?;
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
            let (url, public_key) = begin_link(app, None)?;
            Ok(json!({"url":url,"publicKey":public_key}))
        }
        "public_account_poll" => {
            if !finish_link(app)? {
                return Ok(json!({"linked":false}));
            }
            start(app)?;
            Ok(json!({"linked":true}))
        }
        "public_account_remote_enable" => {
            let scope: ShareScope =
                serde_json::from_value(args.clone()).map_err(|_| "Invalid sharing scope")?;
            let id = enable(app, scope)?;
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

/// Starts device linking: registers this host's name and E2EE key with the account service and returns the
/// confirmation page URL and the public key. The pending code stays in memory until [`finish_link`] succeeds.
pub fn begin_link(app: &AppCtx, name: Option<&str>) -> Result<(String, String), String> {
    let origin = "https://velaterm.com".to_string();
    let keys = ServerKeys::load_or_create(&app.data_dir()?)?;
    let name = name
        .map(str::to_string)
        .or_else(sysinfo::System::host_name)
        .unwrap_or_else(|| "VelaTerm".into());
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
    Ok((url, keys.public_key_b64().to_string()))
}

/// Polls the pending link once. Returns true after the account confirmed this device and the credential was
/// saved; the caller decides whether to start sharing in this process.
pub fn finish_link(app: &AppCtx) -> Result<bool, String> {
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
        return Ok(false);
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
    Ok(true)
}

/// Signs this device out of the account service and deletes the local credential and share records.
pub fn unlink(app: &AppCtx) -> Result<(), String> {
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
    Ok(())
}

/// Account link state without starting any service: `None` when this device was never linked, otherwise
/// the account service's answer plus the local device ID.
pub fn link_status(app: &AppCtx) -> Result<Option<Value>, String> {
    let Ok(config) = load(app) else { return Ok(None) };
    let mut status = request(
        &config.origin,
        "/api/device-link/host/status",
        Some(&config.token),
        None,
    )?;
    status["deviceId"] = json!(config.device_id);
    status["shares"] = json!(config.shares.values().map(|s| &s.scope.scope).collect::<Vec<_>>());
    Ok(Some(status))
}

/// Makes sure the whole workspace is shared with the account, so a freshly linked headless host shows up
/// in the Remote list without any further step. Returns the grant ID.
pub fn ensure_workspace_share(app: &AppCtx) -> Result<String, String> {
    let existing = load(app)?
        .shares
        .into_iter()
        .find(|(_, share)| share.scope.scope == "machine")
        .map(|(id, _)| id);
    match existing {
        Some(id) => Ok(id),
        None => enable(
            app,
            ShareScope {
                scope: "machine".into(),
                target_id: None,
                push_authority: None,
            },
        ),
    }
}

/// Creates a remote access grant for `scope` on the account service and records it locally.
fn enable(app: &AppCtx, scope: ShareScope) -> Result<String, String> {
    let _lock = FILE_LOCK
        .lock()
        .map_err(|_| "Sharing configuration unavailable")?;
    let mut config = load(app)?;
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
    Ok(id.to_string())
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
    // Serialize the remote snapshot and its acknowledgement with local grant mutations.
    // A snapshot fetched before an enable must never remove that newly created grant.
    let _lock = FILE_LOCK.lock().map_err(|_| "Sharing unavailable")?;
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

    /// Only the grant owner, recorded first, counts as the host's own account; invitees and scopes without
    /// a tunnel identity do not.
    #[test]
    fn owner_is_the_first_allowed_account() {
        let dir = std::env::temp_dir().join(format!("vlx-share-owner-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = crate::db::Db::open(&dir.join("test.db")).unwrap();
        let share = "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa";
        let config = json!({
            "origin": "https://velaterm.com",
            "deviceId": "11111111-1111-1111-1111-111111111111",
            "token": "device-token",
            "shares": {share: {"scope": "project", "targetId": "p", "secret": "s", "allowedAccounts": ["owner", "guest"]}}
        });
        std::fs::write(dir.join(CONFIG), config.to_string()).unwrap();
        let app = AppCtx::Headless(std::sync::Arc::new(HeadlessHost::new(dir.clone(), db)));
        let owner = share_scope_for(&app, share, "owner").unwrap();
        let guest = share_scope_for(&app, share, "guest").unwrap();
        assert!(visitor_is_owner(&app, &owner));
        assert!(!visitor_is_owner(&app, &guest));
        let anonymous = ShareScope { scope: "project".into(), target_id: Some("p".into()), push_authority: None };
        assert!(!visitor_is_owner(&app, &anonymous));
        drop(app);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn grant_sync_does_not_erase_a_concurrent_enable() {
        use std::sync::mpsc;
        use std::thread;
        let dir = std::env::temp_dir().join(format!("vlx-grant-race-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let server = loop {
            let port = 10000 + (uuid::Uuid::new_v4().as_u128() % 39152) as u16;
            if let Ok(server) = tiny_http::Server::http(("127.0.0.1", port)) { break server; }
        };
        let config = Config {
            origin: format!("http://{}", server.server_addr()),
            device_id: "11111111-1111-1111-1111-111111111111".into(),
            token: "synthetic-token".into(), shares: HashMap::new(),
        };
        let db = crate::db::Db::open(&dir.join("test.db")).unwrap();
        let app = AppCtx::Headless(std::sync::Arc::new(HeadlessHost::new(dir.clone(), db)));
        save(&app, &config).unwrap();
        let (requested_tx, requested_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let server_thread = thread::spawn(move || {
            let request = server.recv_timeout(Duration::from_secs(5)).unwrap().unwrap();
            assert_eq!(request.url(), "/api/device-link/host/grants");
            requested_tx.send(()).unwrap();
            release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            request.respond(tiny_http::Response::from_string("[]")).unwrap();
            let mut request = server.recv_timeout(Duration::from_secs(5)).unwrap().unwrap();
            assert_eq!(request.method().as_str(), "PUT");
            assert_eq!(request.url(), "/api/device-link/host/shared-scopes");
            let mut body = String::new();
            request.as_reader().read_to_string(&mut body).unwrap();
            assert_eq!(body, "[]");
            request.respond(tiny_http::Response::from_string("null")).unwrap();
        });
        let sync_app = app.clone();
        let sync_thread = thread::spawn(move || sync_grants(&sync_app, &config));
        requested_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let (writing_tx, writing_rx) = mpsc::channel();
        let (written_tx, written_rx) = mpsc::channel();
        let write_app = app.clone();
        let writer = thread::spawn(move || {
            writing_tx.send(()).unwrap();
            let _lock = FILE_LOCK.lock().unwrap();
            let mut current = load(&write_app).unwrap();
            current.shares.insert("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa".into(), LocalShare {
                scope: ShareScope { scope: "machine".into(), target_id: None, push_authority: None },
                secret: "synthetic-secret".into(), password_hash: None,
                allowed_accounts: vec!["synthetic-owner".into()],
            });
            save(&write_app, &current).unwrap();
            written_tx.send(()).unwrap();
        });
        writing_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        // Let the old implementation commit its new grant while the earlier GET is still pending.
        let write_finished_before_snapshot = written_rx.recv_timeout(Duration::from_millis(100)).is_ok();
        release_tx.send(()).unwrap();
        sync_thread.join().unwrap().unwrap();
        writer.join().unwrap();
        server_thread.join().unwrap();
        let retained = load(&app).unwrap().shares.contains_key("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa");
        drop(app);
        std::fs::remove_dir_all(dir).unwrap();
        assert!(retained, "an older grant snapshot erased the newly enabled grant");
        assert!(!write_finished_before_snapshot, "grant mutations must wait for the complete synchronization");
    }

}

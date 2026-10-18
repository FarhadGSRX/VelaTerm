//! Desktop side of full access over the account relay.
//!
//! A normal account Remote window loads its page from velaterm.com, which also terminates TLS. That is fine
//! for chat-only shares, but a relay able to rewrite the page could capture a session with terminal access.
//! For full access the desktop therefore serves its own bundled frontend from a loopback proxy and uses the
//! relay only as a transport:
//!
//! - static assets come from this binary (so the host must run the same version);
//! - `/api/*` and `/ws` are forwarded to `https://velaterm.com/r/<grant>/…` with an account session minted
//!   from this device's credential (browser ticket → session cookie, the same exchange a Remote window does);
//! - the first WebSocket frame, the frontend's `e2ee_hello`, is extended with this device's key and a proof
//!   bound to the frontend's ephemeral E2EE key (see `full_access`). The host upgrades the connection only if
//!   the owner approved this device on the host.
//!
//! The loopback listener is guarded by a per-window secret: the window enters once through a one-time code,
//! which sets an HttpOnly cookie, and every other request must carry it. Other local processes that find the
//! port get 403 instead of a proof-bearing tunnel.

use super::e2ee::ServerKeys;
use super::full_access;
use crate::host::AppCtx;
use axum::body::{Body, Bytes};
use axum::extract::ws::{Message as LocalMessage, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::io::Read;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message as RelayMessage;

/// Session cookies the account service sets for signed-in browsers (see `AccountAccess` in vlx-term-server):
/// the production name on HTTPS, and the development name a local account service uses over plain HTTP.
const ACCOUNT_COOKIES: &[&str] = &["__Host-velaterm-account", "velaterm-account-dev"];
/// Cookie carrying the per-window secret on the loopback proxy.
const PROXY_COOKIE: &str = "vlx_remote_proxy";

/// Request headers forwarded to the relay; everything else (cookies, origin, hop-by-hop) stays local.
const FORWARD_REQUEST: &[&str] = &[
    "content-type", "accept", "accept-language", "range", "if-none-match", "if-modified-since", "x-request-id",
];
/// Response headers passed back to the window.
const FORWARD_RESPONSE: &[&str] = &[
    "content-type", "content-disposition", "content-range", "accept-ranges", "etag", "cache-control",
    "last-modified", "x-request-id",
    // Artifact previews depend on their policy headers; dropping them in transit would let a page go online.
    "content-security-policy", "x-content-type-options", "referrer-policy", "access-control-allow-origin",
];

/// An account session on the relay for one grant, renewed from the device credential when it expires.
pub struct RelaySession {
    app: AppCtx,
    origin: String,
    device_id: String,
    grant_id: String,
    cookie: Mutex<String>,
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(15))
        .timeout_read(Duration::from_secs(90))
        .redirects(0)
        .build()
}

/// ureq reports non-2xx statuses as errors; the proxy wants them as ordinary responses.
fn response(result: Result<ureq::Response, ureq::Error>) -> Result<ureq::Response, String> {
    match result {
        Ok(r) | Err(ureq::Error::Status(_, r)) => Ok(r),
        Err(e) => Err(format!("Cannot reach the account service: {e}")),
    }
}

impl RelaySession {
    /// Mints a session for `grant_id` on `device_id` with this desktop's device credential.
    pub fn open(app: &AppCtx, device_id: &str, grant_id: &str) -> Result<Arc<Self>, String> {
        let origin = super::public_relay::account_origin(app).ok_or("Link this device to your account first")?;
        let session = Arc::new(Self {
            app: app.clone(),
            origin,
            device_id: device_id.to_string(),
            grant_id: grant_id.to_string(),
            cookie: Mutex::new(String::new()),
        });
        session.login()?;
        Ok(session)
    }

    fn login(&self) -> Result<(), String> {
        let url = super::public_relay::browser_ticket_url(&self.app, &self.device_id, Some(&self.grant_id))?;
        let ticket = url
            .split_once("#ticket=")
            .map(|(_, t)| t.to_string())
            .filter(|t| !t.is_empty())
            .ok_or("Invalid browser ticket")?;
        let reply = response(
            agent()
                .post(&format!("{}/api/device-link/browser-session", self.origin))
                .set("Origin", &self.origin)
                .set("Content-Type", "application/json")
                .send_string(&json!({"ticket": ticket}).to_string()),
        )?;
        if reply.status() != 200 {
            return Err("The account service rejected this device".into());
        }
        let cookie = reply
            .all("set-cookie")
            .into_iter()
            .filter_map(|c| c.split(';').next())
            .find(|c| ACCOUNT_COOKIES.iter().any(|name| c.trim_start().starts_with(&format!("{name}="))))
            .map(|c| c.trim().to_string())
            .ok_or("The account service did not start a session")?;
        *self.cookie.lock().map_err(|_| "Session unavailable")? = cookie;
        Ok(())
    }

    fn cookie(&self) -> String {
        self.cookie.lock().map(|c| c.clone()).unwrap_or_default()
    }

    fn relay_url(&self, path_and_query: &str) -> String {
        format!("{}/r/{}{}", self.origin, self.grant_id, path_and_query)
    }

    /// Sends one request to the relay, signing in again once when the session has expired.
    fn send(&self, method: &str, path_and_query: &str, headers: &[(String, String)], body: &[u8]) -> Result<ureq::Response, String> {
        let attempt = || {
            let mut request = agent().request(method, &self.relay_url(path_and_query)).set("Cookie", &self.cookie());
            for (name, value) in headers {
                request = request.set(name, value);
            }
            response(if body.is_empty() { request.call() } else { request.send_bytes(body) })
        };
        let first = attempt()?;
        if first.status() != 401 {
            return Ok(first);
        }
        self.login()?;
        attempt()
    }

    /// Asks the host whether this device will get full access, and which version and E2EE key answer.
    pub fn probe(&self, device_key: &str) -> Result<Value, String> {
        let name = sysinfo::System::host_name().unwrap_or_else(|| "VelaTerm".into());
        let headers = vec![
            ("Accept".to_string(), "application/json".to_string()),
            ("x-vlx-device-key".to_string(), device_key.to_string()),
            ("x-vlx-device-name".to_string(), name),
        ];
        let reply = self.send("GET", "/api/mode", &headers, &[])?;
        if reply.status() != 200 {
            return Err("The remote device is not available".into());
        }
        let mut body = String::new();
        reply
            .into_reader()
            .take(64 * 1024)
            .read_to_string(&mut body)
            .map_err(|_| "Cannot read the remote device's answer")?;
        serde_json::from_str(&body).map_err(|_| "Invalid answer from the remote device".into())
    }

    fn ws_url(&self) -> String {
        let origin = self
            .origin
            .strip_prefix("https://")
            .map(|rest| format!("wss://{rest}"))
            .or_else(|| self.origin.strip_prefix("http://").map(|rest| format!("ws://{rest}")))
            .unwrap_or_else(|| self.origin.clone());
        format!("{origin}/r/{}/ws", self.grant_id)
    }
}

struct ProxyState {
    relay: Arc<RelaySession>,
    device: Arc<ServerKeys>,
    host_key: String,
    secret: String,
    enter_code: Mutex<Option<String>>,
}

/// A running loopback proxy. Dropping it or calling [`RemoteProxy::stop`] shuts the listener down.
pub struct RemoteProxy {
    /// URL the window opens first; it trades the one-time code for the proxy cookie.
    pub entry_url: String,
    shutdown: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
}

impl RemoteProxy {
    pub fn stop(&self) {
        if let Some(tx) = self.shutdown.lock().ok().and_then(|mut s| s.take()) {
            let _ = tx.send(());
        }
    }
}

impl Drop for RemoteProxy {
    fn drop(&mut self) {
        self.stop();
    }
}

fn random() -> String {
    format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple())
}

/// Starts the loopback proxy for a relay session whose host reported `host_key` as its E2EE key.
pub fn start(relay: Arc<RelaySession>, device: Arc<ServerKeys>, host_key: String) -> Result<RemoteProxy, String> {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).map_err(|e| format!("Cannot open a local port: {e}"))?;
    listener.set_nonblocking(true).map_err(|e| format!("Cannot open a local port: {e}"))?;
    let port = listener.local_addr().map_err(|e| format!("Cannot open a local port: {e}"))?.port();
    let enter_code = random();
    let state = Arc::new(ProxyState {
        relay,
        device,
        host_key,
        secret: random(),
        enter_code: Mutex::new(Some(enter_code.clone())),
    });
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    std::thread::Builder::new()
        .name("vlx-remote-proxy".into())
        .spawn(move || {
            let Ok(rt) = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build() else {
                crate::diagnostic_warn!("[remote-proxy] failed to start runtime");
                return;
            };
            rt.block_on(async move {
                let Ok(listener) = tokio::net::TcpListener::from_std(listener) else { return };
                let router = Router::new()
                    .route("/__vlx/enter/:code", get(enter))
                    .route("/ws", get(ws_handler))
                    .route("/api/*rest", axum::routing::any(forward_http))
                    .fallback(static_handler)
                    .layer(axum::middleware::from_fn_with_state(state.clone(), guard))
                    .with_state(state);
                let _ = axum::serve(listener, router)
                    .with_graceful_shutdown(async move {
                        let _ = rx.await;
                    })
                    .await;
            });
        })
        .map_err(|e| format!("Cannot start the local proxy: {e}"))?;
    Ok(RemoteProxy {
        entry_url: format!("http://127.0.0.1:{port}/__vlx/enter/{enter_code}"),
        shutdown: Mutex::new(Some(tx)),
    })
}

fn has_proxy_cookie(headers: &HeaderMap, secret: &str) -> bool {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|c| c.trim().split_once('='))
        .any(|(name, value)| name == PROXY_COOKIE && value == secret)
}

/// Admits only the window that entered through the one-time code.
async fn guard(
    State(state): State<Arc<ProxyState>>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    if request.uri().path().starts_with("/__vlx/enter/") || has_proxy_cookie(request.headers(), &state.secret) {
        return next.run(request).await;
    }
    (StatusCode::FORBIDDEN, "Forbidden").into_response()
}

async fn enter(State(state): State<Arc<ProxyState>>, Path(code): Path<String>) -> Response {
    let valid = state
        .enter_code
        .lock()
        .ok()
        .and_then(|mut slot| slot.take_if(|expected| *expected == code))
        .is_some();
    if !valid {
        return (StatusCode::FORBIDDEN, "Forbidden").into_response();
    }
    let cookie = format!("{PROXY_COOKIE}={}; HttpOnly; SameSite=Strict; Path=/", state.secret);
    let mut response = (StatusCode::FOUND, [(header::LOCATION, "/")]).into_response();
    if let Ok(value) = HeaderValue::from_str(&cookie) {
        response.headers_mut().insert(header::SET_COOKIE, value);
    }
    response
}

async fn static_handler(headers: HeaderMap, uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };
    if let Some(res) = super::static_assets::serve(path, path, super::mime_for(path), &headers).await {
        return res;
    }
    super::static_assets::serve("index.html", "index.html", "text/html; charset=utf-8", &headers)
        .await
        .unwrap_or_else(|| (StatusCode::NOT_FOUND, "Frontend assets not found").into_response())
}

/// Forwards `/api/*` to the relay, streaming the response body so downloads are not buffered in memory.
async fn forward_http(State(state): State<Arc<ProxyState>>, method: Method, uri: Uri, headers: HeaderMap, body: Bytes) -> Response {
    let path = uri.path_and_query().map(|p| p.as_str().to_string()).unwrap_or_else(|| "/".into());
    let forwarded: Vec<(String, String)> = FORWARD_REQUEST
        .iter()
        .filter_map(|name| headers.get(*name).and_then(|v| v.to_str().ok()).map(|v| (name.to_string(), v.to_string())))
        .collect();
    let (head_tx, head_rx) = tokio::sync::oneshot::channel::<Result<(u16, Vec<(String, String)>), String>>();
    let (chunk_tx, chunk_rx) = tokio::sync::mpsc::channel::<Result<Bytes, std::io::Error>>(8);
    let relay = state.relay.clone();
    tokio::task::spawn_blocking(move || {
        let reply = match relay.send(method.as_str(), &path, &forwarded, &body) {
            Ok(reply) => reply,
            Err(e) => {
                let _ = head_tx.send(Err(e));
                return;
            }
        };
        let status = reply.status();
        let headers = FORWARD_RESPONSE
            .iter()
            .filter_map(|name| reply.header(name).map(|v| (name.to_string(), v.to_string())))
            .collect();
        if head_tx.send(Ok((status, headers))).is_err() {
            return;
        }
        let mut reader = reply.into_reader();
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if chunk_tx.blocking_send(Ok(Bytes::copy_from_slice(&buf[..n]))).is_err() {
                        break;
                    }
                }
                Err(e) => {
                    let _ = chunk_tx.blocking_send(Err(e));
                    break;
                }
            }
        }
    });
    let (status, headers) = match head_rx.await {
        Ok(Ok(head)) => head,
        Ok(Err(e)) => return (StatusCode::BAD_GATEWAY, e).into_response(),
        Err(_) => return StatusCode::BAD_GATEWAY.into_response(),
    };
    let stream = futures_util::stream::unfold(chunk_rx, |mut rx| async move { rx.recv().await.map(|chunk| (chunk, rx)) });
    let mut response = Response::new(Body::from_stream(stream));
    *response.status_mut() = StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY);
    for (name, value) in headers {
        if let (Ok(name), Ok(value)) = (header::HeaderName::from_bytes(name.as_bytes()), HeaderValue::from_str(&value)) {
            response.headers_mut().insert(name, value);
        }
    }
    response
}

async fn ws_handler(State(state): State<Arc<ProxyState>>, ws: WebSocketUpgrade) -> Response {
    ws.on_upgrade(move |socket| pump(state, socket))
}

/// Adds the device key and proof to the frontend's `e2ee_hello`; any other frame passes unchanged.
fn with_proof(state: &ProxyState, frame: String) -> String {
    let Ok(mut hello) = serde_json::from_str::<Value>(&frame) else { return frame };
    if hello["type"] != "e2ee_hello" {
        return frame;
    }
    let Some(client_pub) = hello["publicKeyB64"].as_str().map(str::to_string) else { return frame };
    match full_access::sign_proof(&state.device, &state.host_key, &client_pub, &state.relay.grant_id) {
        Ok(proof) => {
            hello["devicePublicKey"] = json!(state.device.public_key_b64());
            hello["deviceProof"] = json!(proof);
            hello.to_string()
        }
        Err(_) => frame,
    }
}

async fn connect_relay(
    relay: &Arc<RelaySession>,
) -> Result<tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>, String> {
    let _ = tokio_rustls::rustls::crypto::aws_lc_rs::default_provider().install_default();
    let dial = |cookie: String| {
        let url = relay.ws_url();
        async move {
            let mut request = url.into_client_request().map_err(|e| e.to_string())?;
            request
                .headers_mut()
                .insert("Cookie", cookie.parse().map_err(|_| "invalid session".to_string())?);
            tokio_tungstenite::connect_async(request).await.map(|(socket, _)| socket).map_err(|e| e.to_string())
        }
    };
    match dial(relay.cookie()).await {
        Ok(socket) => Ok(socket),
        Err(_) => {
            // The session may have expired; mint a new one and try once more.
            let renew = relay.clone();
            tokio::task::spawn_blocking(move || renew.login())
                .await
                .map_err(|_| "session renewal failed".to_string())??;
            dial(relay.cookie()).await
        }
    }
}

async fn pump(state: Arc<ProxyState>, local: WebSocket) {
    let Ok(upstream) = connect_relay(&state.relay).await else {
        crate::diagnostic_warn!("[remote-proxy] cannot open the relay WebSocket");
        return;
    };
    let (mut local_tx, mut local_rx) = local.split();
    let (mut relay_tx, mut relay_rx) = upstream.split();
    let mut first = true;
    loop {
        tokio::select! {
            incoming = local_rx.next() => {
                let message = match incoming {
                    Some(Ok(LocalMessage::Text(text))) => {
                        let text = if first { with_proof(&state, text) } else { text };
                        first = false;
                        RelayMessage::Text(text)
                    }
                    Some(Ok(LocalMessage::Binary(bytes))) => RelayMessage::Binary(bytes),
                    Some(Ok(LocalMessage::Ping(_) | LocalMessage::Pong(_))) => continue,
                    _ => break,
                };
                if relay_tx.send(message).await.is_err() {
                    break;
                }
            }
            incoming = relay_rx.next() => {
                let message = match incoming {
                    Some(Ok(RelayMessage::Text(text))) => LocalMessage::Text(text),
                    Some(Ok(RelayMessage::Binary(bytes))) => LocalMessage::Binary(bytes),
                    Some(Ok(RelayMessage::Ping(payload))) => {
                        let _ = relay_tx.send(RelayMessage::Pong(payload)).await;
                        continue;
                    }
                    Some(Ok(RelayMessage::Pong(_) | RelayMessage::Frame(_))) => continue,
                    _ => break,
                };
                if local_tx.send(message).await.is_err() {
                    break;
                }
            }
        }
    }
    let _ = local_tx.send(LocalMessage::Close(None)).await;
    let _ = relay_tx.send(RelayMessage::Close(None)).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::HeadlessHost;

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("vlx-remote-proxy-{name}-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn free_port() -> u16 {
        std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap().local_addr().unwrap().port()
    }

    const GRANT: &str = "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa";
    const DEVICE: &str = "22222222-2222-2222-2222-222222222222";

    /// Stand-in for velaterm.com: the browser-ticket exchange, then `/r/<grant>/…` forwarded to the host's
    /// share server with the headers the real tunnel injects.
    fn fake_relay(host_port: u16) -> u16 {
        use axum::extract::Request;
        use axum::routing::post;
        let port = free_port();
        let origin = format!("http://127.0.0.1:{port}");
        let authed = |headers: &HeaderMap| {
            headers.get(header::COOKIE).and_then(|v| v.to_str().ok()).is_some_and(|c| c.contains("velaterm-account-dev=session-1"))
        };
        let ticket_origin = origin.clone();
        let router = Router::new()
            .route("/api/device-link/host/browser-ticket", post(move |headers: HeaderMap| async move {
                if headers.get("authorization").and_then(|v| v.to_str().ok()) != Some("Bearer device-token") {
                    return StatusCode::UNAUTHORIZED.into_response();
                }
                axum::Json(json!({"url": format!("{ticket_origin}/account/client-login#ticket=ticket-1")})).into_response()
            }))
            .route("/api/device-link/browser-session", post(|body: String| async move {
                if !body.contains("ticket-1") {
                    return StatusCode::UNAUTHORIZED.into_response();
                }
                ([(header::SET_COOKIE, "velaterm-account-dev=session-1; HttpOnly; Path=/")], axum::Json(json!({"destination":"/"}))).into_response()
            }))
            .route("/r/:grant/ws", get(move |headers: HeaderMap, ws: WebSocketUpgrade| async move {
                if !authed(&headers) {
                    return StatusCode::UNAUTHORIZED.into_response();
                }
                ws.on_upgrade(move |local| async move {
                    let mut request = format!("ws://127.0.0.1:{host_port}/ws").into_client_request().unwrap();
                    for (k, v) in [("x-vlx-tunnel-secret", "tunnel"), ("x-vlx-share", GRANT), ("x-vlx-account", "acct-1")] {
                        request.headers_mut().insert(k, v.parse().unwrap());
                    }
                    let (upstream, _) = tokio_tungstenite::connect_async(request).await.unwrap();
                    let (mut ltx, mut lrx) = local.split();
                    let (mut utx, mut urx) = upstream.split();
                    loop {
                        tokio::select! {
                            m = lrx.next() => match m { Some(Ok(LocalMessage::Text(t))) => { let _ = utx.send(RelayMessage::Text(t)).await; } _ => break },
                            m = urx.next() => match m { Some(Ok(RelayMessage::Text(t))) => { let _ = ltx.send(LocalMessage::Text(t)).await; } Some(Ok(RelayMessage::Ping(_))) => {} _ => break },
                        }
                    }
                }).into_response()
            }))
            .route("/r/:grant/api/*rest", get(move |request: Request| async move {
                if !authed(request.headers()) {
                    return StatusCode::UNAUTHORIZED.into_response();
                }
                let path = request.uri().path().splitn(4, '/').nth(3).unwrap_or("").to_string();
                let headers: Vec<(String, String)> = request.headers().iter()
                    .filter(|(k, _)| k.as_str().starts_with("x-vlx-device"))
                    .map(|(k, v)| (k.to_string(), v.to_str().unwrap().to_string())).collect();
                let body = tokio::task::spawn_blocking(move || {
                    let mut req = ureq::get(&format!("http://127.0.0.1:{host_port}/{path}"))
                        .set("x-vlx-tunnel-secret", "tunnel").set("x-vlx-share", GRANT).set("x-vlx-account", "acct-1");
                    for (k, v) in &headers { req = req.set(k, v); }
                    req.call().unwrap().into_string().unwrap()
                }).await.unwrap();
                ([(header::CONTENT_TYPE, "application/json")], body).into_response()
            }));
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
            rt.block_on(async move {
                let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await.unwrap();
                axum::serve(listener, router).await.unwrap();
            });
        });
        std::thread::sleep(Duration::from_millis(200));
        port
    }

    /// The whole desktop path against a real share server behind a stand-in relay: session minting, the probe,
    /// the one-time entry, the cookie guard, and a WebSocket whose plain `e2ee_hello` leaves the proxy carrying
    /// a device proof that upgrades the connection.
    #[test]
    fn proxy_upgrades_an_approved_device_end_to_end() {
        let host_dir = temp_dir("host");
        let desktop_dir = temp_dir("desktop");
        let client_dir = temp_dir("client");
        let host_db = crate::db::Db::open(&host_dir.join("test.db")).unwrap();
        let host_app = AppCtx::Headless(Arc::new(HeadlessHost::new(host_dir.clone(), host_db)));
        let host_config = json!({"origin":"https://velaterm.com","deviceId":"11111111-1111-1111-1111-111111111111",
            "token":"host-token","shares":{GRANT:{"scope":"machine","secret":"s","allowedAccounts":["acct-1"]}}});
        std::fs::write(host_dir.join("vlx-public-sharing.json"), host_config.to_string()).unwrap();
        let device = Arc::new(ServerKeys::load_or_create(&desktop_dir).unwrap());
        full_access::save(&host_dir, &full_access::FullAccessConfig { enabled: true, devices: vec![full_access::DeviceKey {
            public_key: device.public_key_b64().into(), name: "desktop".into(), at: 0 }] }).unwrap();
        let host_port = free_port();
        let web = super::super::WebServer::new();
        web.start(host_app.clone(), super::super::StartAuth::Tunnel { secret: "tunnel".into() }, Some(host_port),
            super::super::ServeMode::ShareTunnel).unwrap();
        let relay_port = fake_relay(host_port);

        let desktop_config = json!({"origin":format!("http://127.0.0.1:{relay_port}"),"deviceId":DEVICE,"token":"device-token","shares":{}});
        std::fs::write(desktop_dir.join("vlx-public-sharing.json"), desktop_config.to_string()).unwrap();
        let desktop_db = crate::db::Db::open(&desktop_dir.join("test.db")).unwrap();
        let desktop_app = AppCtx::Headless(Arc::new(HeadlessHost::new(desktop_dir.clone(), desktop_db)));

        let relay = RelaySession::open(&desktop_app, DEVICE, GRANT).unwrap();
        let probe = relay.probe(device.public_key_b64()).unwrap();
        assert_eq!(probe["access"], "full");
        assert_eq!(probe["version"], env!("CARGO_PKG_VERSION"));
        let host_key = probe["e2eeKey"].as_str().unwrap().to_string();

        let proxy = start(relay, device.clone(), host_key.clone()).unwrap();
        let base = proxy.entry_url.split("/__vlx/").next().unwrap().to_string();
        let no_redirect = ureq::AgentBuilder::new().redirects(0).build();
        assert_eq!(response(no_redirect.get(&format!("{base}/api/mode")).call()).unwrap().status(), 403);
        let entered = response(no_redirect.get(&proxy.entry_url).call()).unwrap();
        assert_eq!(entered.status(), 302);
        let cookie = entered.header("set-cookie").unwrap().split(';').next().unwrap().to_string();
        assert_eq!(response(no_redirect.get(&proxy.entry_url).call()).unwrap().status(), 403, "entry code is single-use");
        let mode = response(no_redirect.get(&format!("{base}/api/mode")).set("Cookie", &cookie).call()).unwrap();
        assert_eq!(mode.status(), 200);
        assert_eq!(serde_json::from_str::<Value>(&mode.into_string().unwrap()).unwrap()["share"], true);
        let index = response(no_redirect.get(&format!("{base}/")).set("Cookie", &cookie).call()).unwrap();
        assert_ne!(index.status(), 403);

        let client = ServerKeys::load_or_create(&client_dir).unwrap();
        let cipher = client.derive(&host_key).unwrap();
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let (authenticated, reply) = rt.block_on(async {
            let mut request = format!("{}/ws", base.replace("http://", "ws://")).into_client_request().unwrap();
            request.headers_mut().insert("Cookie", cookie.parse().unwrap());
            let (mut ws, _) = tokio_tungstenite::connect_async(request).await.unwrap();
            ws.send(RelayMessage::Text(json!({"type":"e2ee_hello","publicKeyB64":client.public_key_b64()}).to_string())).await.unwrap();
            let Some(Ok(RelayMessage::Text(ready))) = ws.next().await else { panic!("no ready") };
            assert!(ready.contains("e2ee_ready"));
            ws.send(RelayMessage::Text(cipher.encrypt_text(r#"{"type":"e2ee_auth","deviceToken":""}"#).unwrap())).await.unwrap();
            let mut decode = |t: String| serde_json::from_slice::<Value>(&cipher.decrypt_text(&t).unwrap()).unwrap();
            let Some(Ok(RelayMessage::Text(t))) = ws.next().await else { panic!("no auth") };
            let authenticated = decode(t);
            let invoke = json!({"t":"invoke","id":9,"cmd":"system_stats","args":{}}).to_string();
            ws.send(RelayMessage::Text(cipher.encrypt_text(&invoke).unwrap())).await.unwrap();
            loop {
                let Some(Ok(RelayMessage::Text(t))) = ws.next().await else { panic!("closed") };
                let frame = decode(t);
                if frame["t"] == "reply" && frame["id"] == 9 {
                    return (authenticated, frame);
                }
            }
        });
        assert_eq!(authenticated["access"], "full");
        assert_eq!(reply["ok"], true);

        proxy.stop();
        web.stop();
        drop((host_app, desktop_app));
        for dir in [host_dir, desktop_dir, client_dir] {
            std::fs::remove_dir_all(dir).unwrap();
        }
    }

    #[test]
    fn proxy_cookie_must_match_exactly() {
        let mut headers = HeaderMap::new();
        headers.insert(header::COOKIE, HeaderValue::from_static("a=1; vlx_remote_proxy=secret; b=2"));
        assert!(has_proxy_cookie(&headers, "secret"));
        assert!(!has_proxy_cookie(&headers, "secre"));
        let mut other = HeaderMap::new();
        other.insert(header::COOKIE, HeaderValue::from_static("vlx_remote_proxy_x=secret"));
        assert!(!has_proxy_cookie(&other, "secret"));
        assert!(!has_proxy_cookie(&HeaderMap::new(), "secret"));
    }
}

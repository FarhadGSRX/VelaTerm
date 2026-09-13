//! Outbound public account tunnel.
//!
//! The host dials the relay over a WebSocket authenticated with its device token and multiplexes the share
//! server's traffic over it: HTTP requests and responses in both directions, and WebSocket message streams
//! for the application socket. The relay terminates the visitor's TLS and forwards here; the visitor's
//! conversation content stays end-to-end encrypted above this layer, so the relay sees ciphertext only.
//!
//! Wire format inside binary WebSocket messages:
//!
//! ```text
//! byte 0    frame type
//! bytes 1-4 stream id, big endian
//! bytes 5-  payload
//! ```
//!
//! Directions:
//!
//! - relay to host: OPEN_HTTP, HTTP_DATA, HTTP_END, OPEN_WS, WS_TEXT, WS_BINARY, CLOSE, PING
//! - host to relay: RESPONSE_HEAD, RESPONSE_DATA, RESPONSE_END, WS_TEXT, WS_BINARY, CLOSE, PONG, ERROR
//!
//! Exactly one of OPEN_HTTP or OPEN_WS starts a stream; the relay strips the public URL prefix so the host
//! always sees root paths such as `/api/mode` and `/ws`.

use crate::host::AppCtx;
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;

const TYPE_OPEN_HTTP: u8 = 1;
const TYPE_HTTP_DATA: u8 = 2;
const TYPE_HTTP_END: u8 = 3;
const TYPE_OPEN_WS: u8 = 4;
const TYPE_WS_TEXT: u8 = 5;
const TYPE_WS_BINARY: u8 = 6;
const TYPE_CLOSE: u8 = 7;
const TYPE_RESPONSE_HEAD: u8 = 8;
const TYPE_RESPONSE_DATA: u8 = 9;
const TYPE_RESPONSE_END: u8 = 10;
const TYPE_PING: u8 = 11;
const TYPE_PONG: u8 = 12;
const TYPE_ERROR: u8 = 13;

/// Largest request body the tunnel accepts from the relay. Larger bodies are rejected instead of buffered.
const MAX_REQUEST_BODY: usize = 16 * 1024 * 1024;
/// Response chunks are capped so one large asset cannot monopolize the tunnel.
const RESPONSE_CHUNK: usize = 256 * 1024;

/// Starts the tunnel thread for one account link. The caller has already validated the configuration.
pub fn start(
    app: AppCtx,
    origin: String,
    token: String,
    share_port: u16,
    tunnel_secret: String,
) {
    // A share host serves loopback HTTP, so it may never initialize the HTTPS listener.
    // With both ring and aws-lc enabled, rustls needs a provider before the outbound WSS handshake.
    let _ = tokio_rustls::rustls::crypto::aws_lc_rs::default_provider().install_default();
    std::thread::Builder::new()
        .name("share-tunnel".into())
        .spawn(move || {
            let rt = match tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    crate::diagnostic_warn!("failed to start share tunnel runtime: {e}");
                    return;
                }
            };
            rt.block_on(run(app, origin, token, share_port, tunnel_secret));
        })
        .map_err(|e| crate::diagnostic_warn!("failed to start share tunnel: {e}"))
        .ok();
}

/// Reconnect loop: dial, serve until the socket drops, then back off and retry while the account stays
/// linked. A missing or replaced configuration file ends the loop so a logged-out device stops dialing.
async fn run(app: AppCtx, origin: String, token: String, share_port: u16, tunnel_secret: String) {
    let mut backoff = 1u64;
    loop {
        match connect_once(&app, &origin, &token, share_port, &tunnel_secret).await {
            Ok(()) => backoff = 1,
            Err(e) => {
                crate::diagnostic_warn!("share tunnel disconnected: {e}");
            }
        }
        if !config_matches(&app, &token) {
            return;
        }
        tokio::time::sleep(Duration::from_secs(backoff)).await;
        backoff = (backoff * 2).min(30);
    }
}

/// Whether the on-disk account link still matches this tunnel's token.
fn config_matches(app: &AppCtx, token: &str) -> bool {
    super::public_relay::config_token(app).as_deref() == Some(token)
}

/// One tunnel session. Returns when the socket closes or the dial fails.
async fn connect_once(
    app: &AppCtx,
    origin: &str,
    token: &str,
    share_port: u16,
    tunnel_secret: &str,
) -> Result<(), String> {
    let url = format!("{}/api/relay/host/tunnel", ws_origin(origin));
    let mut request = url
        .into_client_request()
        .map_err(|e| format!("invalid tunnel URL: {e}"))?;
    request.headers_mut().insert(
        "Authorization",
        format!("Bearer {token}")
            .parse()
            .map_err(|_| "invalid device token".to_string())?,
    );
    let (socket, _) = tokio_tungstenite::connect_async(request)
        .await
        .map_err(|e| format!("cannot reach the account service: {e}"))?;
    // Each new connection starts without shared scopes; confirm the host's current snapshot immediately.
    let sync_app = app.clone();
    if !matches!(tokio::task::spawn_blocking(move || super::public_relay::sync(&sync_app)).await, Ok(Ok(()))) {
        crate::diagnostic_warn!("failed to synchronize shared scopes after connecting");
    }
    let (mut sink, mut stream) = socket.split();
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<Message>();
    let writer = tokio::spawn(async move {
        while let Some(message) = out_rx.recv().await {
            if sink.send(message).await.is_err() {
                break;
            }
        }
    });
    let local = Arc::new(LocalTarget {
        port: share_port,
        secret: tunnel_secret.to_string(),
        http: ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(60))
            .build(),
    });
    let mut streams: std::collections::HashMap<u32, StreamState> = std::collections::HashMap::new();
    let result = loop {
        let message = match stream.next().await {
            Some(Ok(message)) => message,
            Some(Err(e)) => break Err(format!("tunnel socket error: {e}")),
            None => break Ok(()),
        };
        match message {
            Message::Binary(bytes) => {
                let Some((kind, stream_id, payload)) = parse_frame(&bytes) else {
                    continue;
                };
                match kind {
                    TYPE_OPEN_HTTP => {
                        let meta: Value = match serde_json::from_slice(payload) {
                            Ok(meta) => meta,
                            Err(_) => {
                                send_error(&out_tx, stream_id, "Invalid request metadata");
                                continue;
                            }
                        };
                        let body = meta
                            .get("body")
                            .and_then(Value::as_str)
                            .and_then(|b| {
                                use base64::Engine;
                                base64::engine::general_purpose::STANDARD.decode(b).ok()
                            })
                            .unwrap_or_default();
                        let complete = meta.get("body").is_some();
                        streams.insert(
                            stream_id,
                            StreamState::Http {
                                meta,
                                body,
                                complete,
                            },
                        );
                        if complete {
                            if let Some(state) = streams.remove(&stream_id) {
                                spawn_http(&local, stream_id, state, out_tx.clone());
                            }
                        }
                    }
                    TYPE_HTTP_DATA => {
                        let mut overflow = false;
                        if let Some(StreamState::Http { body, .. }) = streams.get_mut(&stream_id) {
                            if body.len() + payload.len() > MAX_REQUEST_BODY {
                                overflow = true;
                            } else {
                                body.extend_from_slice(payload);
                            }
                        }
                        if overflow {
                            streams.remove(&stream_id);
                            send_error(&out_tx, stream_id, "Request body too large");
                        }
                    }
                    TYPE_HTTP_END => {
                        let completed = match streams.get_mut(&stream_id) {
                            Some(StreamState::Http { complete, .. }) => {
                                *complete = true;
                                true
                            }
                            _ => false,
                        };
                        if completed {
                            if let Some(state) = streams.remove(&stream_id) {
                                spawn_http(&local, stream_id, state, out_tx.clone());
                            }
                        }
                    }
                    TYPE_OPEN_WS => {
                        let meta: Value = serde_json::from_slice(payload).unwrap_or(Value::Null);
                        let (to_local, rx) = mpsc::unbounded_channel();
                        streams.insert(stream_id, StreamState::Ws { to_local });
                        spawn_ws(&local, stream_id, meta, rx, out_tx.clone());
                    }
                    TYPE_WS_TEXT | TYPE_WS_BINARY => {
                        if let Some(StreamState::Ws { to_local }) = streams.get(&stream_id) {
                            let _ = to_local.send((kind == TYPE_WS_TEXT, payload.to_vec()));
                        }
                    }
                    TYPE_CLOSE => {
                        if let Some(state) = streams.remove(&stream_id) {
                            state.close();
                        }
                    }
                    TYPE_PING => {
                        let _ = out_tx.send(Message::Binary(frame(TYPE_PONG, stream_id, &[])));
                    }
                    _ => {}
                }
            }
            Message::Ping(payload) => {
                let _ = out_tx.send(Message::Pong(payload));
            }
            Message::Close(_) => break Ok(()),
            _ => {}
        }
    };
    writer.abort();
    result
}

/// Per-stream state kept by the reader loop. HTTP bodies buffer until the request is complete; WebSocket
/// streams forward message chunks into the local task.
enum StreamState {
    Http {
        meta: Value,
        body: Vec<u8>,
        complete: bool,
    },
    Ws {
        to_local: mpsc::UnboundedSender<(bool, Vec<u8>)>,
    },
}

impl StreamState {
    fn close(&self) {
        if let StreamState::Ws { to_local, .. } = self {
            let _ = to_local.send((false, Vec::new()));
        }
    }
}

/// Shared local target description for spawned stream tasks.
struct LocalTarget {
    port: u16,
    secret: String,
    http: ureq::Agent,
}

/// Forwards one completed HTTP request to the share server and streams the response back. The blocking
/// client runs on the blocking pool so it never occupies the tunnel runtime.
fn spawn_http(
    local: &Arc<LocalTarget>,
    stream_id: u32,
    state: StreamState,
    out_tx: mpsc::UnboundedSender<Message>,
) {
    let StreamState::Http { meta, body, .. } = state else {
        return;
    };
    let local = local.clone();
    tokio::task::spawn_blocking(move || {
        let method = meta["method"].as_str().unwrap_or("GET").to_string();
        let path = meta["path"].as_str().unwrap_or("/");
        let url = format!("http://127.0.0.1:{}{}", local.port, path);
        let mut request = local.http.request(&method, &url);
        if let Some(headers) = meta["headers"].as_object() {
            for (name, value) in headers {
                let Some(value) = value.as_str() else { continue };
                // Host, content length, hop-by-hop headers, and the browser's Accept-Encoding belong to the
                // local hop: the blocking client negotiates and transparently decodes its own compression,
                // while a forwarded Accept-Encoding would leave the body encoded but the header stripped.
                if matches!(
                    name.as_str(),
                    "host" | "content-length" | "connection" | "transfer-encoding" | "accept-encoding"
                ) {
                    continue;
                }
                request = request.set(name, value);
            }
        }
        request = request
            .set("x-vlx-tunnel-secret", &local.secret)
            .set(
                "x-vlx-share",
                meta["grantId"].as_str().unwrap_or_default(),
            )
            .set(
                "x-vlx-account",
                meta["accountId"].as_str().unwrap_or_default(),
            );
        let response = if body.is_empty() {
            request.call()
        } else {
            request.send_bytes(&body)
        };
        let response = match response {
            Ok(response) => response,
            // The blocking client reports 4xx/5xx as errors; the response is still a real answer to stream.
            Err(ureq::Error::Status(_, response)) => response,
            Err(e) => {
                send_error(&out_tx, stream_id, &format!("local request failed: {e}"));
                return;
            }
        };
        let status = response.status();
        let headers: serde_json::Map<String, Value> = response
            .headers_names()
            .into_iter()
            .filter_map(|name| {
                response
                    .header(&name)
                    .map(|value| (name, json!(value)))
            })
            .collect();
        let head = json!({"status": status, "headers": headers});
        let _ = out_tx.send(Message::Binary(frame(
            TYPE_RESPONSE_HEAD,
            stream_id,
            head.to_string().as_bytes(),
        )));
        use std::io::Read;
        let mut reader = response.into_reader();
        let mut buffer = vec![0u8; RESPONSE_CHUNK];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => {
                    let _ = out_tx.send(Message::Binary(frame(
                        TYPE_RESPONSE_DATA,
                        stream_id,
                        &buffer[..read],
                    )));
                }
                Err(e) => {
                    send_error(&out_tx, stream_id, &format!("response read failed: {e}"));
                    break;
                }
            }
        }
        let _ = out_tx.send(Message::Binary(frame(TYPE_RESPONSE_END, stream_id, &[])));
    });
}

/// Opens one local WebSocket to the share server and pipes messages both ways until either side closes.
/// `to_local_rx` carries frames arriving from the relay; local output is pushed straight to the tunnel.
fn spawn_ws(
    local: &Arc<LocalTarget>,
    stream_id: u32,
    meta: Value,
    mut to_local_rx: mpsc::UnboundedReceiver<(bool, Vec<u8>)>,
    out_tx: mpsc::UnboundedSender<Message>,
) {
    let local = local.clone();
    tokio::spawn(async move {
        let path = meta["path"].as_str().unwrap_or("/ws");
        let url = format!("ws://127.0.0.1:{}{}", local.port, path);
        let mut request = match url.into_client_request() {
            Ok(request) => request,
            Err(_) => {
                send_error(&out_tx, stream_id, "Invalid WebSocket path");
                return;
            }
        };
        {
            let headers = request.headers_mut();
            let _ = headers.insert(
                "x-vlx-tunnel-secret",
                local.secret.parse().expect("secret is ASCII"),
            );
            if let Some(grant) = meta["grantId"].as_str() {
                if let Ok(value) = grant.parse() {
                    let _ = headers.insert("x-vlx-share", value);
                }
            }
            if let Some(account) = meta["accountId"].as_str() {
                if let Ok(value) = account.parse() {
                    let _ = headers.insert("x-vlx-account", value);
                }
            }
        }
        let (socket, _) = match tokio_tungstenite::connect_async(request).await {
            Ok(socket) => socket,
            Err(e) => {
                send_error(&out_tx, stream_id, &format!("local WebSocket failed: {e}"));
                return;
            }
        };
        let (mut sink, mut stream) = socket.split();
        loop {
            tokio::select! {
                incoming = stream.next() => {
                    match incoming {
                        Some(Ok(Message::Text(text))) => {
                            let _ = out_tx.send(Message::Binary(frame(TYPE_WS_TEXT, stream_id, text.as_bytes())));
                        }
                        Some(Ok(Message::Binary(bytes))) => {
                            let _ = out_tx.send(Message::Binary(frame(TYPE_WS_BINARY, stream_id, &bytes)));
                        }
                        Some(Ok(Message::Ping(payload))) => {
                            let _ = sink.send(Message::Pong(payload)).await;
                        }
                        Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                        _ => {}
                    }
                }
                outgoing = to_local_rx.recv() => {
                    match outgoing {
                        Some((true, bytes)) => {
                            if sink
                                .send(Message::Text(String::from_utf8_lossy(&bytes).into_owned()))
                                .await
                                .is_err()
                            {
                                break;
                            }
                        }
                        Some((false, bytes)) if !bytes.is_empty() => {
                            if sink.send(Message::Binary(bytes)).await.is_err() {
                                break;
                            }
                        }
                        Some((false, _)) => {
                            let _ = sink.close().await;
                            break;
                        }
                        None => break,
                    }
                }
            }
        }
        let _ = out_tx.send(Message::Binary(frame(TYPE_CLOSE, stream_id, &[])));
    });
}

fn send_error(out_tx: &mpsc::UnboundedSender<Message>, stream_id: u32, message: &str) {
    let payload = json!({"message": message}).to_string();
    let _ = out_tx.send(Message::Binary(frame(TYPE_ERROR, stream_id, payload.as_bytes())));
}

/// Converts an `https://host` origin into the `wss://host` WebSocket origin.
fn ws_origin(origin: &str) -> String {
    if let Some(rest) = origin.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = origin.strip_prefix("http://") {
        format!("ws://{rest}")
    } else {
        origin.to_string()
    }
}

fn frame(kind: u8, stream_id: u32, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(5 + payload.len());
    out.push(kind);
    out.extend_from_slice(&stream_id.to_be_bytes());
    out.extend_from_slice(payload);
    out
}

fn parse_frame(data: &[u8]) -> Option<(u8, u32, &[u8])> {
    if data.len() < 5 {
        return None;
    }
    let stream_id = u32::from_be_bytes([data[1], data[2], data[3], data[4]]);
    Some((data[0], stream_id, &data[5..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The wire format is the contract with the relay; a drift here silently breaks every share visit.
    #[test]
    fn frames_round_trip_and_origins_convert() {
        let bytes = frame(TYPE_OPEN_HTTP, 7, b"{\"path\":\"/api/mode\"}");
        let (kind, stream, payload) = parse_frame(&bytes).unwrap();
        assert_eq!(kind, TYPE_OPEN_HTTP);
        assert_eq!(stream, 7);
        assert_eq!(payload, b"{\"path\":\"/api/mode\"}");
        assert!(parse_frame(&[1, 2, 3]).is_none());
        assert_eq!(ws_origin("https://velaterm.com"), "wss://velaterm.com");
        assert_eq!(ws_origin("http://127.0.0.1:18799"), "ws://127.0.0.1:18799");
    }
}

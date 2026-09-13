//! Loopback Web server that serves the real application to public share visitors.
//!
//! The account tunnel forwards relay traffic into this instance; nothing else can reach it. It is a normal
//! [`WebServer`] in [`ServeMode::ShareTunnel`], so static assets, `/ws`, E2EE, dispatch, and the share scope
//! filtering all run through the same code path as LAN remote access. Requests must carry the per-launch
//! tunnel secret, which is generated here and handed to the tunnel client in memory only.
//!
//! The instance starts lazily the first time the account tunnel needs it and stays up for the process
//! lifetime; a stopped tunnel simply leaves it idle on loopback.

use std::sync::{Mutex, OnceLock};

use super::{ServeMode, StartAuth, WebServer};
use crate::host::AppCtx;

struct ShareServer {
    server: WebServer,
    port: u16,
    secret: String,
}

static SERVER: OnceLock<Mutex<Option<ShareServer>>> = OnceLock::new();

/// Starts the share server on first use and returns its loopback port together with the tunnel secret.
pub fn ensure(app: &AppCtx) -> Result<(u16, String), String> {
    let slot = SERVER.get_or_init(|| Mutex::new(None));
    let mut guard = slot.lock().map_err(|_| "Share server unavailable")?;
    if let Some(existing) = guard.as_ref() {
        return Ok((existing.port, existing.secret.clone()));
    }
    let port = free_loopback_port()?;
    let secret = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    let server = WebServer::new();
    server.start(
        app.clone(),
        StartAuth::Tunnel {
            secret: secret.clone(),
        },
        Some(port),
        ServeMode::ShareTunnel,
    )?;
    *guard = Some(ShareServer {
        server,
        port,
        secret: secret.clone(),
    });
    Ok((port, secret))
}

/// Stops the share server, used when the account is unlinked. A later link starts a fresh instance.
pub fn stop() {
    let Some(slot) = SERVER.get() else { return };
    let Ok(mut guard) = slot.lock() else { return };
    if let Some(existing) = guard.take() {
        existing.server.stop();
    }
}

/// Picks a free loopback port. The bind is released before the Web server binds it again, the same small
/// race every dynamic-port picker accepts.
fn free_loopback_port() -> Result<u16, String> {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0))
        .map_err(|e| format!("failed to pick a share-server port: {e}"))?;
    listener
        .local_addr()
        .map(|addr| addr.port())
        .map_err(|e| format!("failed to read the share-server port: {e}"))
}

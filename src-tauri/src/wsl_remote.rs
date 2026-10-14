//! Local WSL workspaces. The Linux backend owns agents, files, credentials, history, and its stable port.
//! Windows only provisions a signed binary and opens the existing browser-mode desktop window.
#![cfg_attr(not(all(windows, feature = "gui")), allow(dead_code))]

use std::collections::HashMap;
use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use serde::{Deserialize, Serialize};

const SCRIPT: &str = include_str!("../resources/wsl-service.sh");
const LAST_DISTRIBUTION: &str = "wsl-last-distribution";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Options {
    supported: bool,
    distributions: Vec<String>,
    selected: Option<String>,
    error: Option<String>,
}

pub fn options(app: &crate::host::AppCtx) -> Result<Options, String> {
    #[cfg(all(windows, feature = "gui"))]
    if matches!(app, crate::host::AppCtx::Tauri(_)) {
        let (distributions, error) = match distributions() { Ok(items) => (items, None), Err(error) => (Vec::new(), Some(error)) };
        let conn = app.db().conn.lock().map_err(|e| e.to_string())?;
        let settings = crate::db::repo::get_app_settings(&conn)?;
        let selected = settings.get(LAST_DISTRIBUTION).filter(|v| distributions.contains(v))
            .cloned().or_else(|| distributions.first().cloned());
        return Ok(Options { supported: true, distributions, selected, error });
    }
    let _ = app;
    Ok(Options { supported: false, distributions: Vec::new(), selected: None, error: None })
}

fn distributions() -> Result<Vec<String>, String> {
    let mut cmd = crate::host::command("wsl.exe");
    cmd.args(["--list", "--quiet"]);
    let output = capture(cmd, Vec::new(), Duration::from_secs(20))?;
    // wsl.exe uses UTF-16 for distribution enumeration, but Linux child commands use UTF-8.
    #[cfg(any(windows, test))]
    { Ok(crate::pty::manager::parse_wsl_distros(&output)) }
    #[cfg(not(any(windows, test)))]
    { let _ = output; Err("WSL connections require the Windows desktop app".into()) }
}

/// Bounded child I/O: neither an unresponsive WSL instance nor a full pipe can freeze a native command.
fn capture(mut cmd: Command, input: Vec<u8>, timeout: Duration) -> Result<Vec<u8>, String> {
    let mut child = cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped())
        .spawn().map_err(|e| format!("Cannot run WSL: {e}"))?;
    let mut stdin = child.stdin.take().ok_or("WSL input pipe is unavailable")?;
    let stdout = child.stdout.take().ok_or("WSL output pipe is unavailable")?;
    let stderr = child.stderr.take().ok_or("WSL error pipe is unavailable")?;
    let (tx, rx) = std::sync::mpsc::channel();
    for (index, pipe) in [(0, Box::new(stdout) as Box<dyn Read + Send>), (1, Box::new(stderr) as Box<dyn Read + Send>)] {
        let tx = tx.clone();
        std::thread::spawn(move || {
            let mut output = Vec::new();
            let result = pipe.take(65536).read_to_end(&mut output);
            let _ = tx.send((index, result.map(|_| output)));
        });
    }
    std::thread::spawn(move || { let _ = stdin.write_all(&input); });
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(25)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("WSL did not respond in time. Check the distribution and try again".into());
            }
        }
    };
    let mut output = [Vec::new(), Vec::new()];
    for _ in 0..2 {
        let (index, result) = rx.recv_timeout(Duration::from_secs(2))
            .map_err(|_| "WSL left an output pipe open")?;
        output[index] = result.map_err(|e| format!("Cannot read WSL output: {e}"))?;
    }
    if !status.success() {
        // Never include stdout: status output contains the workspace's private login credential.
        let error = String::from_utf8_lossy(&output[1]);
        let detail = error.trim().chars().take(500).collect::<String>();
        return Err(if detail.is_empty() { "WSL command failed. Check that the distribution is installed and can start".into() }
            else { format!("WSL: {detail}") });
    }
    Ok(std::mem::take(&mut output[0]))
}

fn quote(value: &str) -> String { format!("'{}'", value.replace('\'', "'\\''")) }

#[derive(Clone, Deserialize)]
struct Service {
    port: u16,
    password: String,
    pid: u32,
    stamp: String,
    version: String,
    running: bool,
    #[serde(default)]
    created: bool,
}

fn parse_service(bytes: &[u8]) -> Result<Service, String> {
    let service: Service = serde_json::from_slice(bytes).map_err(|_| "Invalid WSL service response")?;
    if (service.port != 0 && !(10000..=49151).contains(&service.port))
        || (!service.password.is_empty() && (service.password.len() < 32 || service.password.len() > 128
            || !service.password.bytes().all(|b| b.is_ascii_alphanumeric())))
        || service.stamp.is_empty() || !service.stamp.bytes().all(|b| b.is_ascii_digit())
        || service.version.is_empty() || !service.version.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        || (service.running && (service.pid < 2 || service.port == 0 || service.password.is_empty()))
    { return Err("Invalid WSL service state".into()); }
    Ok(service)
}

pub struct Connection {
    pub session: String,
    pub distribution: String,
    namespace: String,
    service: Mutex<Service>,
    operation: Mutex<()>,
}

impl Connection {
    fn run(&self, operation: &str, values: &[(&str, &str)]) -> Result<Vec<u8>, String> {
        let mut script = format!("operation={}\nnamespace={}\nversion={}\n", quote(operation), quote(&self.namespace), quote(crate::VERSION));
        for (key, value) in values { script.push_str(&format!("{key}={}\n", quote(value))); }
        script.push_str(SCRIPT);
        let mut command = crate::host::command("wsl.exe");
        command.args(["--distribution", &self.distribution, "--exec", "sh", "-s"]);
        capture(command, script.into_bytes(), Duration::from_secs(120))
    }

    fn status(&self) -> Result<Service, String> { parse_service(&self.run("status", &[])?) }

    pub fn address(&self) -> Result<(u16, String), String> {
        let service = self.service.lock().map_err(|e| e.to_string())?;
        Ok((service.port, service.password.clone()))
    }

    fn stop(&self, service: &Service) -> Result<(), String> {
        self.run("stop", &[("expected_pid", &service.pid.to_string()), ("expected_stamp", &service.stamp)])?;
        Ok(())
    }

    fn choose_port(&self, previous: u16, fixed: bool) -> Result<u16, String> {
        for attempt in 0..48 {
            let candidate = if attempt == 0 && previous != 0 { previous } else {
                10000 + (uuid::Uuid::new_v4().as_u128() % 39152) as u16
            };
            if fixed && candidate != previous { break; }
            // Windows and WSL can have independent listeners; both sides must be free.
            let Ok(_reservation) = std::net::TcpListener::bind(("127.0.0.1", candidate)) else { continue; };
            if self.run("port", &[("candidate", &candidate.to_string())])? == b"free\n" {
                return Ok(candidate);
            }
        }
        Err("The WSL workspace port is occupied. Close this window and connect again".into())
    }

    fn start(&self, previous: &Service, fixed_port: bool) -> Result<Service, String> {
        if previous.running {
            if previous.version != crate::VERSION {
                return Err("A different VelaTerm server version is running in this WSL workspace. Stop it from its existing connection before upgrading".into());
            }
            wait_ready(previous)?;
            return Ok(previous.clone());
        }
        let port = self.choose_port(previous.port, fixed_port)?;
        let password = uuid::Uuid::new_v4().simple().to_string();
        let started = parse_service(&self.run("start", &[("candidate", &port.to_string()), ("new_password", &password)])?)?;
        if started.version != crate::VERSION { return Err("A different WSL server version is already running".into()); }
        if let Err(error) = wait_ready(&started) {
            // A reused instance belongs to other windows too; only roll back the process we just started.
            if started.created { let _ = self.stop(&started); }
            return Err(error);
        }
        let mut service = self.status()?;
        if !service.running { return Err("The WSL server exited during startup; check its server.log".into()); }
        service.created = started.created;
        Ok(service)
    }
}

fn wait_ready(service: &Service) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(30);
    let agent = ureq::AgentBuilder::new().try_proxy_from_env(false).redirects(0).build();
    loop {
        // Authenticate before trusting a loopback listener; a port collision must never open an unrelated app.
        let response = agent.post(&format!("http://127.0.0.1:{}/api/login", service.port))
            .timeout(Duration::from_secs(2)).set("Content-Type", "application/json")
            .send_string(&serde_json::json!({"password":service.password}).to_string());
        if let Ok(response) = response {
            if response.status() == 200 {
                let body = response.into_string().unwrap_or_default();
                if serde_json::from_str::<serde_json::Value>(&body).ok()
                    .and_then(|v| v.get("token").and_then(|t| t.as_str()).map(|s| !s.is_empty())).unwrap_or(false)
                { return Ok(()); }
            }
        }
        if Instant::now() >= deadline { break; }
        std::thread::sleep(Duration::from_millis(400));
    }
    Err("Cannot connect to the WSL server through localhost. Check WSL localhost forwarding, port conflicts, and the workspace server.log".into())
}

fn connections() -> &'static Mutex<HashMap<String, Arc<Connection>>> {
    static CONNECTIONS: OnceLock<Mutex<HashMap<String, Arc<Connection>>>> = OnceLock::new();
    CONNECTIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

#[cfg(feature = "gui")]
pub fn connect(app: &crate::host::AppCtx, distribution: &str, restart_existing: bool, progress: &dyn Fn(&str, Option<u8>)) -> Result<Arc<Connection>, String> {
    if !cfg!(windows) { return Err("WSL connections require the Windows desktop app".into()); }
    if !distributions()?.iter().any(|d| d == distribution) { return Err("The selected WSL distribution is no longer installed".into()); }
    let data_dir = app.data_dir()?;
    let namespace = data_dir.file_name().and_then(|s| s.to_str()).ok_or("Invalid application data directory")?.to_owned();
    if !namespace.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b)) { return Err("Invalid WSL workspace namespace".into()); }
    let connection = Arc::new(Connection {
        session: uuid::Uuid::new_v4().to_string(), distribution: distribution.to_owned(), namespace,
        service: Mutex::new(Service { port: 0, password: String::new(), pid: 0, stamp: "0".into(), version: crate::VERSION.into(), running: false, created: false }),
        operation: Mutex::new(()),
    });
    progress("probe", None);
    let mut previous = connection.status()?;
    let upgrade = previous.running && previous.version != crate::VERSION;
    if upgrade && !restart_existing { return Err("wsl_version_running".into()); }
    if !previous.running || upgrade {
        let mut probe = crate::host::command("wsl.exe");
        probe.args(["--distribution", distribution, "--exec", "uname", "-m"]);
        let arch = capture(probe, Vec::new(), Duration::from_secs(20))?;
        let arch = match String::from_utf8_lossy(&arch).trim() {
            "x86_64" => "x86_64", "aarch64" | "arm64" => "aarch64",
            _ => return Err("This WSL architecture has no published VelaTerm server".into()),
        };
        progress("supply", None);
        let platform = crate::server_supply::platform_key("linux", arch);
        let supplied = crate::server_supply::ensure_supplied(&data_dir, crate::VERSION, &platform, &|pct| progress("supply", Some(pct)))?;
        progress("transfer", None);
        connection.run("install", &[("source", &supplied.bin.to_string_lossy()), ("sha", &supplied.sha256)])?;
    }
    if upgrade { connection.stop(&previous)?; previous = connection.status()?; }
    progress("start", None);
    let service = connection.start(&previous, false)?;
    *connection.service.lock().map_err(|e| e.to_string())? = service;
    connections().lock().map_err(|e| e.to_string())?.insert(connection.session.clone(), connection.clone());
    // Remember only a successful choice; stale selections never override the current backend catalog.
    let conn = app.db().conn.lock().map_err(|e| e.to_string())?;
    let _ = crate::db::repo::set_app_settings(&conn, &HashMap::from([(LAST_DISTRIBUTION.to_owned(), distribution.to_owned())]));
    Ok(connection)
}

pub fn reconnect(session: &str) -> Result<(), String> {
    let connection = connections().lock().map_err(|e| e.to_string())?.get(session).cloned().ok_or("WSL connection is closed")?;
    let _guard = connection.operation.try_lock().map_err(|_| "WSL connection is busy")?;
    let previous = connection.status()?;
    let (port, password) = connection.address()?;
    if previous.port != port || previous.password != password { return Err("The WSL workspace configuration changed. Open a new connection".into()); }
    let service = connection.start(&previous, true)?;
    *connection.service.lock().map_err(|e| e.to_string())? = service;
    Ok(())
}

pub fn disconnect(session: &str, stop: bool) -> Result<(), String> {
    let connection = connections().lock().map_err(|e| e.to_string())?.get(session).cloned();
    if let Some(connection) = connection {
        let _guard = connection.operation.lock().map_err(|e| e.to_string())?;
        if stop {
            let service = connection.service.lock().map_err(|e| e.to_string())?.clone();
            connection.stop(&service)?;
        }
        connections().lock().map_err(|e| e.to_string())?.remove(session);
    }
    Ok(())
}

pub fn connection_failed(session: &str) -> Result<(), String> {
    let connection = connections().lock().map_err(|e| e.to_string())?.get(session).cloned();
    let stop = connection.as_ref().and_then(|c| c.service.lock().ok().map(|s| s.created)).unwrap_or(false);
    disconnect(session, stop)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_state_rejects_invalid_ports_credentials_and_process_identity() {
        let valid = serde_json::json!({"port":28473,"password":"a".repeat(32),"pid":42,"stamp":"98765","version":"0.2.4","running":true});
        assert!(parse_service(valid.to_string().as_bytes()).is_ok());
        for (key, value) in [("port", serde_json::json!(8799)), ("password", serde_json::json!("short")),
            ("pid", serde_json::json!(1)), ("stamp", serde_json::json!("1;kill")), ("version", serde_json::json!("../other"))] {
            let mut invalid = valid.clone(); invalid[key] = value;
            assert!(parse_service(invalid.to_string().as_bytes()).is_err(), "{key}");
        }
        assert!(parse_service(b"not JSON").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn shell_values_round_trip_without_command_execution() {
        let value = "Ubuntu 'quoted' $HOME $(exit 9) `exit 8`\\path\nnext";
        let mut command = Command::new("sh"); command.arg("-s");
        let script = format!("value={}\nprintf '%s' \"$value\"", quote(value));
        assert_eq!(capture(command, script.into_bytes(), Duration::from_secs(2)).unwrap(), value.as_bytes());
    }

    #[cfg(unix)]
    #[test]
    fn child_timeout_is_bounded_and_error_does_not_expose_stdout() {
        let mut command = Command::new("sh"); command.args(["-c", "printf secret; exit 1"]);
        assert!(!capture(command, Vec::new(), Duration::from_secs(2)).unwrap_err().contains("secret"));
        let mut command = Command::new("sleep"); command.arg("5");
        let started = Instant::now();
        assert!(capture(command, Vec::new(), Duration::from_millis(100)).is_err());
        assert!(started.elapsed() < Duration::from_secs(2));
    }
}

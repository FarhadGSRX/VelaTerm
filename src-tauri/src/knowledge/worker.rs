//! Reuse the pinned upstream SDK over private pipes; no extra service or listener is created.
use super::*;
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::sync::atomic::{AtomicBool,Ordering};
use std::time::{Duration, Instant};

struct Worker { child: Child, input: ChildStdin, output: Receiver<Value> }
impl Drop for Worker {
    fn drop(&mut self) {
        #[cfg(unix)] unsafe { libc::kill(-(self.child.id() as i32), libc::SIGKILL); }
        #[cfg(windows)] { let _ = crate::host::command("taskkill").args(["/PID", &self.child.id().to_string(), "/T", "/F"]).stdout(Stdio::null()).stderr(Stdio::null()).status(); }
        let _ = self.child.kill(); let _ = self.child.wait();
    }
}
type Slot = Arc<(Mutex<Option<Worker>>, AtomicBool)>;
fn workers() -> &'static Mutex<HashMap<(PathBuf, String), Slot>> {
    static WORKERS: OnceLock<Mutex<HashMap<(PathBuf, String), Slot>>> = OnceLock::new();
    WORKERS.get_or_init(|| {
        std::thread::spawn(|| loop {
            std::thread::sleep(Duration::from_secs(30));
            if let Ok(slots) = workers().lock() {
                for slot in slots.values() {
                    if let Ok(mut worker) = slot.0.try_lock() {
                        if worker.as_mut().is_some_and(|w| !matches!(w.child.try_wait(),Ok(None))) { worker.take(); }
                    }
                }
            }
        });
        Mutex::default()
    })
}
pub fn stop(app: &AppCtx, root: &str) {
    if let Ok(dir) = app.data_dir() {
        if let Some(slot) = workers().lock().unwrap().remove(&(dir, root.to_owned())) {
            // Do not block the disable command behind an ongoing query; the query observes enabled=0.
            slot.1.store(true,Ordering::SeqCst);
            std::thread::spawn(move || { slot.0.lock().unwrap().take(); });
        }
    }
}
fn spawn(app: &AppCtx, index: &Index) -> Result<Worker> {
    let (node, script) = runtime::executable(app)?;
    let entry = script.parent().and_then(Path::parent).ok_or("knowledge_runtime_missing")?.join("index.js");
    let mut command = crate::host::command(node);
    command.args(["--liftoff-only", "--disable-warning=ExperimentalWarning", "-e", include_str!("bridge.cjs")])
        .arg(entry).arg(&index.root).current_dir(&index.root)
        .env("CODEGRAPH_TELEMETRY", "0").env("DO_NOT_TRACK", "1")
        .env("CODEGRAPH_NO_UPDATE_CHECK", "1").env("CODEGRAPH_NO_DAEMON", "1")
        .env("NO_COLOR", "1").env("CI", "1")
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
    #[cfg(unix)] { use std::os::unix::process::CommandExt; command.process_group(0); }
    let mut child = command.spawn().map_err(|_| "knowledge_process_failed")?;
    let input = child.stdin.take().ok_or("knowledge_process_failed")?;
    let stdout = child.stdout.take().ok_or("knowledge_process_failed")?;
    let (tx, output) = mpsc::sync_channel(1);
    let ctx = app.clone(); let root = index.root.clone();
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        loop {
            let mut bytes = Vec::new();
            match reader.by_ref().take(9 * 1024 * 1024).read_until(b'\n', &mut bytes) {
                Ok(0) | Err(_) => break,
                _ if bytes.last() != Some(&b'\n') => break,
                _ => (),
            }
            let Some(payload) = bytes.strip_prefix(b"VLX_KNOWLEDGE ") else { continue; };
            let Ok(value) = serde_json::from_slice::<Value>(payload) else { break; };
            if let Some(event) = value["event"].as_str() {
                if event == "updated" {
                    let s = &value["stats"];
                    let stats = json!({"nodes":s["nodeCount"],"files":s["fileCount"],"edges":s["edgeCount"],"state":"complete"});
                    if let Ok(conn) = ctx.db().conn.lock() {
                        let _ = conn.execute("UPDATE knowledge_indexes SET stats=?1,updated_at=?2 WHERE root=?3 AND enabled=1 AND job_id=''", params![stats.to_string(),now(),root]);
                    }
                    ctx.emit("knowledge://changed", ());
                }
                if event == "watchError" { runtime::audit(&ctx,"system","ERROR","watch_failed",0); }
                continue;
            }
            if tx.send(value).is_err() { break; }
        }
    });
    Ok(Worker { child, input, output })
}
pub fn query(app: &AppCtx, index: &Index, args: &Value) -> Result<Value> {
    if !get(app,&index.id)?.enabled {return Err("knowledge_disabled".into());}
    let key = (app.data_dir()?, index.root.clone());
    let slot = workers().lock().map_err(|_| "knowledge_busy")?.entry(key).or_default().clone();
    let handle=slot;
    let mut slot = handle.0.lock().map_err(|_| "knowledge_busy")?;
    if handle.1.load(Ordering::SeqCst) { return Err("knowledge_disabled".into()); }
    if slot.as_mut().is_some_and(|w| !matches!(w.child.try_wait(), Ok(None))) { slot.take(); }
    if slot.is_none() { *slot = Some(spawn(app,index)?); }
    let worker = slot.as_mut().unwrap();
    let started = Instant::now();
    runtime::audit(app,&index.id,"INFO","query_started",0);
    let result = (|| {
        writeln!(worker.input,"{}",args).map_err(|_| "knowledge_process_failed")?;
        loop {
            match worker.output.recv_timeout(Duration::from_millis(250)) {
                Ok(value) => {
                    if handle.1.load(Ordering::SeqCst) { return Err("knowledge_disabled".into()); }
                    if let Some(error) = value["error"].as_str() { return Err(error.to_string()); }
                    return value.get("result").cloned().ok_or_else(|| "knowledge_query_failed".into());
                },
                Err(mpsc::RecvTimeoutError::Disconnected) => return Err("knowledge_process_failed".into()),
                Err(mpsc::RecvTimeoutError::Timeout) => (),
            }
            if handle.1.load(Ordering::SeqCst) || !get(app,&index.id).is_ok_and(|i| i.enabled) { return Err("knowledge_disabled".into()); }
            if started.elapsed() > Duration::from_secs(1800) { return Err("knowledge_timeout".into()); }
        }
    })();
    if result.is_err() { slot.take(); }
    runtime::audit(app,&index.id,if result.is_ok(){"INFO"}else{"ERROR"},if result.is_ok(){"query_completed"}else{"query_failed"},started.elapsed().as_millis());
    result
}

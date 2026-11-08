//! `vrun`: start a long-running command and wait for it in one call.
//!
//! An agent learns that work has finished only when a command it issued returns. Starting work with
//! `nohup ... &` returns at once, so the agent must write a second command to wait for it — and that
//! second command is written fresh every time. Forgetting it, attaching it to the previous round, or
//! matching the waiting command itself with `pgrep` all end the same way: the work finishes and nobody
//! is told. This command removes that gap by starting the work and waiting for it in a single call.
//!
//! The work runs in its own session (`setsid`) with `SIGHUP` ignored, so closing the conversation does
//! not kill a build that has been running for an hour. Waiting is a plain `wait` on our own child, so
//! there is no pattern to match and nothing to confuse with this process. Every wait is bounded: a
//! timeout returns 124 and says so, rather than leaving the caller asleep forever.

use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const USAGE: &str = "usage: vrun [-t <seconds>] <label> <command> [arguments...]
       vrun --status <label>
Starts the command, waits for it, and exits when it does, so its completion reaches you.
Prints the exit code, the elapsed time and the tail of the log; the command's exit code is passed through.
Waiting is bounded: reaching the timeout exits 124 and leaves the command running.
    -t <seconds>  how long to wait before giving up (default 1800)
    --status      report on a label started earlier";

const DEFAULT_TIMEOUT_SECS: u64 = 1800;
const POLL: Duration = Duration::from_millis(500);
const TAIL_BYTES: u64 = 64 * 1024;
const TAIL_LINES: usize = 20;
/// Reaching the wait limit is not the command failing. 124 is what `timeout(1)` returns, so a caller
/// that already knows that convention reads it correctly.
const TIMEOUT_EXIT: i32 = 124;
/// Written when the waiting run times out, holding the time in ms since the epoch.
const WAITED_FILE: &str = "waited";

struct Parsed {
    label: String,
    timeout: u64,
    command: Vec<String>,
}

enum Parse {
    Help,
    Status(String),
    Run(Parsed),
    Err(String),
}

fn parse(args: &[String]) -> Parse {
    let mut timeout = DEFAULT_TIMEOUT_SECS;
    let mut i = 0;
    while let Some(arg) = args.get(i) {
        match arg.as_str() {
            "--help" | "-h" => return Parse::Help,
            "--status" => {
                return match args.get(i + 1) {
                    Some(label) => Parse::Status(label.clone()),
                    None => Parse::Err("--status needs a label".into()),
                };
            }
            "-t" | "--timeout" => {
                let Some(value) = args.get(i + 1) else {
                    return Parse::Err("-t needs a number of seconds".into());
                };
                match value.parse::<u64>() {
                    Ok(secs) if secs > 0 => timeout = secs,
                    _ => return Parse::Err(format!("Invalid timeout {value}")),
                }
                i += 2;
            }
            other if other.starts_with('-') => return Parse::Err(format!("Unknown option {other}")),
            // The label ends option parsing: everything after it belongs to the command, including its
            // own flags, so `vrun build cargo test --lib` needs no `--` separator.
            _ => break,
        }
    }
    let Some(label) = args.get(i) else {
        return Parse::Err("Specify a label and a command".into());
    };
    if label.trim().is_empty() || label.contains('/') || label.contains('\\') {
        return Parse::Err("The label must be a plain name without path separators".into());
    }
    let command: Vec<String> = args[i + 1..].to_vec();
    if command.is_empty() {
        return Parse::Err("Specify the command to run after the label".into());
    }
    Parse::Run(Parsed { label: label.clone(), timeout, command })
}

/// Where a label's log and receipts live: beside the injected `bin/` directory, so runs sit with the
/// rest of the session's data. Outside a session, fall back to a temporary directory.
fn run_dir(label: &str) -> PathBuf {
    let base = std::env::var_os("VLX_BIN_DIR")
        .map(PathBuf::from)
        .and_then(|bin| bin.parent().map(|dir| dir.join("runs")))
        .unwrap_or_else(|| std::env::temp_dir().join("velaterm-runs"));
    base.join(label)
}

/// Read the last lines of the log without loading a long build's entire output.
fn tail(path: &std::path::Path) -> String {
    tail_lines(path, TAIL_BYTES, TAIL_LINES)
}

/// The last `max_lines` lines within the last `max_bytes` of a file. Shared with the log viewer.
pub(crate) fn tail_lines(path: &std::path::Path, max_bytes: u64, max_lines: usize) -> String {
    let Ok(mut file) = std::fs::File::open(path) else { return String::new() };
    let Ok(len) = file.metadata().map(|m| m.len()) else { return String::new() };
    let from = len.saturating_sub(max_bytes);
    if file.seek(SeekFrom::Start(from)).is_err() { return String::new(); }
    let mut buffer = Vec::new();
    if file.read_to_end(&mut buffer).is_err() { return String::new(); }
    let text = String::from_utf8_lossy(&buffer);
    let lines: Vec<&str> = text.lines().collect();
    lines[lines.len().saturating_sub(max_lines)..].join("\n")
}

/// Tell the VelaTerm instance hosting this session to look at its runs again.
///
/// Best effort and brief: the service also polls, so a lost announcement only delays what the session
/// shows by a few seconds, and a slow one must never hold up the work or its result.
fn announce() {
    let (Ok(url), Ok(session_id), Ok(token)) = (
        std::env::var("VLX_SPAWN_URL"),
        std::env::var("VLX_SESSION_ID"),
        std::env::var("VLX_TOKEN"),
    ) else {
        return;
    };
    let body = serde_json::json!({ "sessionId": session_id }).to_string();
    let _ = super::cli_client::post_brief(&format!("{url}/runs?t={token}"), &body);
}

#[cfg(unix)]
fn detach(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    // Safety: both calls are async-signal-safe and run between fork and exec.
    unsafe {
        command.pre_exec(|| {
            libc::signal(libc::SIGHUP, libc::SIG_IGN);
            // A new session detaches the work from the process group the host may terminate.
            libc::setsid();
            Ok(())
        });
    }
}

#[cfg(windows)]
fn detach(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    command.creation_flags(CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS);
}

fn now_ms() -> u128 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0)
}

/// What `--status` says about a label. Pure so each case can be tested: an exit code wins; otherwise a
/// live PID is running; a gone PID with a marker means the command ended after the waiting `vrun` had
/// given up, so its exit code was never written; a gone PID without one means it was stopped.
fn status_message(label: &str, exit: Option<i32>, pid: Option<u32>, running: bool, waited: Option<&str>) -> String {
    if let Some(code) = exit {
        return format!("vrun: {label} finished with exit code {code}");
    }
    match (pid, running, waited) {
        (None, _, _) => format!("vrun: {label} has no recorded PID"),
        (Some(pid), true, _) => format!("vrun: {label} is still running as PID {pid}"),
        (Some(pid), false, Some(at)) => format!("vrun: {label} (PID {pid}) finished after vrun stopped waiting at {at} (ms since epoch); its exit code was not recorded"),
        (Some(pid), false, None) => format!("vrun: {label} (PID {pid}) recorded no exit code; it was stopped, or vrun did not outlive it"),
    }
}
fn status(label: &str) -> ! {
    let dir = run_dir(label);
    if !dir.exists() {
        eprintln!("vrun: no run recorded for {label}");
        std::process::exit(1);
    }
    let log = dir.join("log");
    let exit = std::fs::read_to_string(dir.join("exit")).ok().and_then(|code| code.trim().parse::<i32>().ok());
    let pid = std::fs::read_to_string(dir.join("pid")).ok().and_then(|text| text.trim().parse::<u32>().ok());
    let running = pid.is_some_and(super::runs::alive);
    let waited = std::fs::read_to_string(dir.join(WAITED_FILE)).ok().map(|text| text.trim().to_string());
    println!("{}", status_message(label, exit, pid, running, waited.as_deref()));
    println!("vrun: log {}", log.display());
    let text = tail(&log);
    if !text.is_empty() { println!("{text}"); }
    std::process::exit(0);
}

pub fn run(args: &[String]) -> ! {
    match parse(&args[args.len().min(2)..]) {
        Parse::Help => {
            println!("{USAGE}");
            std::process::exit(0);
        }
        Parse::Err(message) => {
            eprintln!("vrun: {message}");
            eprintln!("{USAGE}");
            std::process::exit(2);
        }
        Parse::Status(label) => status(&label),
        Parse::Run(parsed) => start(parsed),
    }
}

fn start(parsed: Parsed) -> ! {
    let dir = run_dir(&parsed.label);
    if let Err(error) = std::fs::create_dir_all(&dir) {
        eprintln!("vrun: cannot create {}: {error}", dir.display());
        std::process::exit(2);
    }
    let log_path = dir.join("log");
    // A new run replaces the previous one's receipts, so a stale exit code is never read as this run's,
    // and the previous run's record never names this one's session while it starts.
    let _ = std::fs::remove_file(dir.join(super::runs::META_FILE));
    let _ = std::fs::remove_file(dir.join("exit"));
    let _ = std::fs::remove_file(dir.join(WAITED_FILE));
    let log = match std::fs::File::create(&log_path) {
        Ok(file) => file,
        Err(error) => {
            eprintln!("vrun: cannot write {}: {error}", log_path.display());
            std::process::exit(2);
        }
    };
    let errors = match log.try_clone() {
        Ok(handle) => handle,
        Err(error) => {
            eprintln!("vrun: cannot open the log for the command's errors: {error}");
            std::process::exit(2);
        }
    };
    let mut command = Command::new(&parsed.command[0]);
    command.args(&parsed.command[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(errors));
    detach(&mut command);
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            eprintln!("vrun: cannot start {}: {error}", parsed.command[0]);
            std::process::exit(2);
        }
    };
    let pid = child.id();
    let _ = std::fs::write(dir.join("pid"), pid.to_string());
    // The record that lets the session show this run and stay busy until it ends.
    let meta = super::runs::RunMeta {
        session_id: std::env::var("VLX_SESSION_ID").unwrap_or_default(),
        command: parsed.command.clone(),
        started_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0),
        pid,
    };
    if let Ok(text) = serde_json::to_string(&meta) {
        let _ = std::fs::write(dir.join(super::runs::META_FILE), text);
    }
    announce();
    println!("vrun: {} · PID {pid} · timeout {}s · log {}", parsed.label, parsed.timeout, log_path.display());

    let started = Instant::now();
    let limit = Duration::from_secs(parsed.timeout);
    loop {
        match child.try_wait() {
            Ok(Some(exit)) => {
                let code = exit.code().unwrap_or(1);
                let _ = std::fs::write(dir.join("exit"), code.to_string());
                announce();
                println!("vrun: {} finished · exit code {code} · {}s", parsed.label, started.elapsed().as_secs());
                let text = tail(&log_path);
                if !text.is_empty() { println!("{text}"); }
                std::process::exit(code);
            }
            Ok(None) => {}
            Err(error) => {
                eprintln!("vrun: lost track of PID {pid}: {error}");
                std::process::exit(2);
            }
        }
        if started.elapsed() >= limit {
            println!("vrun: waited {}s without {} finishing. PID {pid} is still running and was not stopped.", parsed.timeout, parsed.label);
            println!("vrun: check on it with `vrun --status {}`; the log is {}", parsed.label, log_path.display());
            let text = tail(&log_path);
            if !text.is_empty() { println!("{text}"); }
            let _ = std::fs::write(dir.join(WAITED_FILE), now_ms().to_string());
            std::process::exit(TIMEOUT_EXIT);
        }
        std::thread::sleep(POLL);
    }
}

#[cfg(test)]
mod tests {
    use super::status_message;

    #[test]
    fn status_messages_cover_every_case() {
        assert!(status_message("b", Some(3), Some(1), false, None).contains("exit code 3"));
        assert!(status_message("b", None, Some(7), true, Some("1")).contains("still running as PID 7"));
        let gone = status_message("b", None, Some(7), false, Some("12345"));
        assert!(gone.contains("stopped waiting at 12345") && gone.contains("not recorded"));
        assert!(status_message("b", None, Some(7), false, None).contains("it was stopped"));
        assert!(status_message("b", None, None, false, None).contains("no recorded PID"));
    }
}

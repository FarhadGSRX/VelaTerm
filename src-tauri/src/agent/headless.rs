//! Running one agent CLI non-interactively to answer a single question.
//!
//! `vrefer --ask` reads another session's transcript and needs it summarized without that transcript
//! passing through the caller's own context. The reading therefore happens in a short-lived agent
//! process: it gets the transcript plus the question, prints an answer, and exits. No session row, no
//! PTY, no window, nothing to clean up.
//!
//! Every agent CLI spells this differently, so [`spec`] is the one table that knows how. Adding an agent
//! means adding one arm there and nothing else. A kind missing from the table simply cannot summarize;
//! whether it can be *read* is a separate question owned by `transcript.rs`.
//!
//! Nothing here talks to a model directly — it shells out to whichever agent CLI the user already has.

use std::io::{Read, Write};
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use crate::models::SessionKind;

/// How the prompt reaches the process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptDelivery {
    /// Appended as the final argument. Every CLI accepts this, but the whole command line has to fit
    /// under the operating system's argument cap, which a long transcript will not.
    Arg { marker: Option<&'static str> },
    /// Written to stdin, which is then closed. `marker`, when set, is the argument that tells the CLI to
    /// read its prompt from stdin. Only claude and codex document this, and only they can therefore take
    /// a transcript of any size.
    Stdin { marker: Option<&'static str> },
}

/// How stdout becomes the one answer returned to `vrefer`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputShape {
    /// The CLI writes only the final answer to stdout.
    Plain,
    /// OpenCode writes JSONL events; concatenate completed text parts from the event stream.
    OpenCodeJson,
}

/// How one agent CLI runs a single non-interactive prompt.
#[derive(Debug, Clone)]
pub struct HeadlessSpec {
    /// Settings and probe key for this kind, matching `executable::resolve` and
    /// `install::locate_installed_bin`.
    pub key: &'static str,
    /// Command name looked up on PATH when no absolute path is configured.
    pub bin: &'static str,
    /// Subcommand and flags, in order, before the prompt.
    pub args: &'static [&'static str],
    pub prompt: PromptDelivery,
    pub output: OutputShape,
}

impl HeadlessSpec {
    /// Whether this CLI can take a prompt too large to pass as an argument.
    pub fn accepts_long_prompt(&self) -> bool {
        matches!(self.prompt, PromptDelivery::Stdin { .. })
    }
}

/// Prompt size beyond which argument delivery is refused.
///
/// macOS caps the whole argument block near 1 MB and Linux is larger but still bounded. Staying well
/// under it leaves room for the flags and the environment, which count against the same limit.
pub const ARG_PROMPT_LIMIT: usize = 128 * 1024;

/// Return how to invoke this agent kind headlessly, or None when it has no usable one-shot mode.
///
/// Flags come from each CLI's own `--help` at the version noted; these interfaces change, so a mismatch
/// later is easiest to diagnose against the version recorded here. Every entry skips interactive
/// approval, because a headless process that stops to ask a question just hangs until it is killed.
pub fn spec(kind: SessionKind) -> Option<HeadlessSpec> {
    match kind {
        // Claude Code 2.1.235: `-p/--print` prints the reply and exits; text is the default format.
        SessionKind::Claude => Some(HeadlessSpec {
            key: "claude",
            bin: "claude",
            args: &["-p", "--permission-mode", "bypassPermissions"],
            prompt: PromptDelivery::Stdin { marker: None },
            output: OutputShape::Plain,
        }),
        // codex-cli 0.146.0: `exec` is the non-interactive subcommand, and `-` makes it read stdin.
        // `--color never` keeps escape sequences out of the answer.
        SessionKind::Codex => Some(HeadlessSpec {
            key: "codex",
            bin: "codex",
            args: &[
                "exec",
                "--sandbox",
                "danger-full-access",
                "--color",
                "never",
            ],
            prompt: PromptDelivery::Stdin { marker: Some("-") },
            output: OutputShape::Plain,
        }),
        // Cursor CLI 2026.07.01: `-p` is headless; `--trust` skips the workspace-trust prompt, which only
        // exists in that mode. Its help documents no stdin path, so the prompt goes in an argument.
        SessionKind::Cursor => Some(HeadlessSpec {
            key: "cursor",
            bin: "cursor-agent",
            args: &["-p", "--output-format", "text", "--force", "--trust"],
            prompt: PromptDelivery::Arg { marker: None },
            output: OutputShape::Plain,
        }),
        // GitHub Copilot CLI 1.0.63: `--allow-all-tools` is required for non-interactive runs, `-s` drops
        // the trailing statistics so stdout is just the answer, and `--no-ask-user` stops it stalling on a
        // question nobody will answer. Its prompt is the value of `-p`, so the argument order differs.
        SessionKind::Copilot => Some(HeadlessSpec {
            key: "copilot",
            bin: "copilot",
            args: &["--allow-all-tools", "--no-ask-user", "-s", "--no-color"],
            prompt: PromptDelivery::Arg { marker: Some("-p") },
            output: OutputShape::Plain,
        }),
        // grok 0.2.118: `-p/--single` is one-shot and `plain` is the default output format.
        SessionKind::Grok => Some(HeadlessSpec {
            key: "grok",
            bin: "grok",
            args: &[
                "--output-format",
                "plain",
                "--permission-mode",
                "bypassPermissions",
            ],
            prompt: PromptDelivery::Arg { marker: Some("-p") },
            output: OutputShape::Plain,
        }),
        // OpenCode 1.18.5 exposes a JSONL event stream in one-shot mode. `decode_output` extracts text
        // parts so progress framing and session metadata never leak into the answer.
        SessionKind::Opencode => Some(HeadlessSpec {
            key: "opencode",
            bin: "opencode",
            args: &["run", "--format", "json", "--pure"],
            prompt: PromptDelivery::Arg { marker: None },
            output: OutputShape::OpenCodeJson,
        }),
        // Pi and OMP both provide print mode. Disable tools, skills and session persistence: a reference
        // summary is a bounded read-only model call and must not create another visible conversation.
        SessionKind::Pi => Some(HeadlessSpec {
            key: "pi",
            bin: "pi",
            args: &[
                "--no-session",
                "--no-tools",
                "--no-skills",
                "--no-extensions",
                "--no-context-files",
            ],
            prompt: PromptDelivery::Arg { marker: Some("-p") },
            output: OutputShape::Plain,
        }),
        SessionKind::Omp => Some(HeadlessSpec {
            key: "omp",
            bin: "omp",
            args: &[
                "--no-session",
                "--no-tools",
                "--no-skills",
                "--no-extensions",
                "--no-rules",
                "--no-title",
            ],
            prompt: PromptDelivery::Arg { marker: Some("-p") },
            output: OutputShape::Plain,
        }),
        // The rest either have no documented one-shot mode or have not been checked on a real machine.
        // Add an arm here once verified against that CLI's own help; nothing else needs to change.
        _ => None,
    }
}

/// A resolved summarizer: which agent, how to invoke it, and where its binary is.
#[derive(Debug, Clone)]
pub struct Summarizer {
    pub kind: SessionKind,
    pub spec: HeadlessSpec,
    /// Absolute path or bare command name to execute.
    pub bin: String,
}

/// Preference order when nothing else decides.
///
/// The two that read stdin come first: they are the only ones that can take a whole transcript, so
/// preferring them means the same choice keeps working as the target session grows.
const PREFERENCE: &[SessionKind] = &[
    SessionKind::Claude,
    SessionKind::Codex,
    SessionKind::Opencode,
    SessionKind::Pi,
    SessionKind::Omp,
    SessionKind::Cursor,
    SessionKind::Copilot,
    SessionKind::Grok,
];

/// Kinds that could answer this prompt, best first.
///
/// An explicit `want` is honored or nothing is: silently substituting another agent for the one the
/// caller named would make `--with` a suggestion rather than a choice. Otherwise the caller's own kind
/// leads, so the work runs on the agent the user already chose and pays for.
///
/// Kept free of any filesystem probing so the ordering can be tested on its own.
fn candidates(
    want: Option<SessionKind>,
    caller: Option<SessionKind>,
    prompt_len: usize,
) -> Vec<SessionKind> {
    let needs_stdin = prompt_len > ARG_PROMPT_LIMIT;
    let usable =
        |kind: SessionKind| spec(kind).is_some_and(|s| !needs_stdin || s.accepts_long_prompt());
    if let Some(want) = want {
        return if usable(want) { vec![want] } else { Vec::new() };
    }
    let mut out: Vec<SessionKind> = Vec::new();
    if let Some(caller) = caller.filter(|k| usable(*k)) {
        out.push(caller);
    }
    for kind in PREFERENCE {
        if usable(*kind) && !out.contains(kind) {
            out.push(*kind);
        }
    }
    out
}

/// Pick a summarizer that is actually installed, or None when none is.
///
/// None is not an error: the caller falls back to returning the transcript itself, which is what
/// `vrefer` did before this existed.
pub fn pick(
    app: &crate::host::AppCtx,
    want: Option<SessionKind>,
    caller: Option<SessionKind>,
    prompt_len: usize,
) -> Option<Summarizer> {
    for kind in candidates(want, caller, prompt_len) {
        let spec = spec(kind)?;
        if let Some(bin) = resolve_bin(app, kind, &spec) {
            return Some(Summarizer { kind, spec, bin });
        }
    }
    None
}

/// Locate this agent's executable: the path configured in settings, then the known install locations,
/// then PATH. Reuses the existing probes rather than adding a third notion of "installed".
fn resolve_bin(
    app: &crate::host::AppCtx,
    kind: SessionKind,
    spec: &HeadlessSpec,
) -> Option<String> {
    debug_assert_eq!(kind.as_str(), spec.key);
    super::executable::resolve(app, kind, None)
        .or_else(|| super::executable::find_on_path(spec.bin))
}

/// Why a headless run produced no answer.
#[derive(Debug)]
pub enum HeadlessError {
    /// A persisted model or effort contains syntax the target CLI cannot safely receive.
    InvalidSelection(String),
    /// The prompt is too large for argument delivery and this CLI cannot read stdin.
    PromptTooLarge { limit: usize, actual: usize },
    /// The process could not be started, usually because the binary is missing.
    Start(String),
    /// The process outlived its deadline and was killed.
    Timeout(Duration),
    /// The process exited nonzero. Carries a trimmed tail of stderr for the caller to report.
    Failed { code: Option<i32>, stderr: String },
    /// The process succeeded but printed nothing usable.
    Empty,
}

impl std::fmt::Display for HeadlessError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidSelection(e) => write!(f, "invalid summarizer settings: {e}"),
            Self::PromptTooLarge { limit, actual } => write!(
                f,
                "prompt is {actual} bytes, over this agent's {limit}-byte argument limit"
            ),
            Self::Start(e) => write!(f, "could not start the summarizer: {e}"),
            Self::Timeout(d) => write!(f, "summarizer timed out after {}s", d.as_secs()),
            Self::Failed { code, stderr } => match code {
                Some(c) => write!(f, "summarizer exited {c}: {stderr}"),
                None => write!(f, "summarizer was terminated: {stderr}"),
            },
            Self::Empty => write!(f, "summarizer produced no output"),
        }
    }
}

/// Longest stderr tail carried back in an error, enough to identify a failure without dumping a log.
const STDERR_TAIL: usize = 600;

/// Run one prompt through an agent CLI and return what it printed.
///
/// Takes the invocation in pieces rather than a `SessionKind` so it can be tested against ordinary
/// commands without calling a model.
///
/// stdout and stderr are drained on their own threads. A single-threaded read would deadlock the moment
/// the child filled the pipe it was not being read from, and a chatty agent fills stderr readily.
pub fn run(
    bin: &str,
    args: &[&str],
    prompt: &str,
    delivery: PromptDelivery,
    cwd: Option<&Path>,
    timeout: Duration,
) -> Result<String, HeadlessError> {
    if matches!(delivery, PromptDelivery::Arg { .. }) && prompt.len() > ARG_PROMPT_LIMIT {
        return Err(HeadlessError::PromptTooLarge {
            limit: ARG_PROMPT_LIMIT,
            actual: prompt.len(),
        });
    }

    let mut command = crate::host::command(bin);
    super::executable::prepare_command(&mut command, bin);
    command.args(args);
    match delivery {
        PromptDelivery::Arg { marker } => {
            if let Some(marker) = marker {
                command.arg(marker);
            }
            command.arg(prompt);
        }
        PromptDelivery::Stdin { marker } => {
            if let Some(marker) = marker {
                command.arg(marker);
            }
        }
    }
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    // The summarizer must not be able to spawn or reference sessions of its own: that would let one
    // reference fan out into a recursion the caller never asked for and cannot see.
    command.env_remove("VLX_SPAWN_URL");
    command.env_remove("VLX_SESSION_ID");
    command.env_remove("VLX_TOKEN");
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        // Its own process group, so the timeout path can take down the helper processes an agent starts
        // rather than only the one this code spawned.
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }

    let started = std::time::Instant::now();
    let mut child = command
        .spawn()
        .map_err(|e| HeadlessError::Start(e.to_string()))?;

    // Writing the prompt gets its own thread: a large one fills the pipe, and the child cannot drain it
    // while this side is still blocked on the write.
    if let PromptDelivery::Stdin { .. } = delivery {
        if let Some(mut stdin) = child.stdin.take() {
            let body = prompt.to_string();
            std::thread::spawn(move || {
                let _ = stdin.write_all(body.as_bytes());
            });
        }
    } else {
        drop(child.stdin.take());
    }

    let out_reader = child.stdout.take().map(|mut out| {
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut buf = String::new();
            let _ = out.read_to_string(&mut buf);
            let _ = sender.send(buf);
        });
        receiver
    });
    let err_reader = child.stderr.take().map(|mut err| {
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut buf = String::new();
            let _ = err.read_to_string(&mut buf);
            let _ = sender.send(buf);
        });
        receiver
    });

    let status = match wait_with_deadline(&mut child, timeout.saturating_sub(started.elapsed())) {
        Some(status) => status,
        None => {
            terminate(&mut child);
            let _ = child.wait();
            return Err(HeadlessError::Timeout(timeout));
        }
    };

    // Helpers may keep inherited pipes open after the main process exits. Collect both streams
    // within the original deadline instead of joining reader threads without a time limit.
    let collect = |reader: Option<std::sync::mpsc::Receiver<String>>| match reader {
        Some(reader) => reader.recv_timeout(timeout.saturating_sub(started.elapsed())),
        None => Ok(String::new()),
    };
    let (stdout, stderr) = match collect(out_reader)
        .and_then(|out| collect(err_reader).map(|err| (out, err)))
    {
        Ok(output) => output,
        Err(_) => {
            terminate(&mut child);
            let _ = child.wait();
            return Err(HeadlessError::Timeout(timeout));
        }
    };

    if !status.success() {
        return Err(HeadlessError::Failed {
            code: status.code(),
            stderr: tail(&stderr, STDERR_TAIL),
        });
    }
    let answer = stdout.trim().to_string();
    if answer.is_empty() {
        return Err(HeadlessError::Empty);
    }
    Ok(answer)
}

/// Run a resolved agent with the model and effort selected in Settings.
///
/// Selection flags are rendered by the same backend-owned mapping used by launch dialogs, so `--model`,
/// Codex config overrides, OpenCode variants and Pi thinking levels cannot drift apart.
pub fn run_selected(
    summarizer: &Summarizer,
    prompt: &str,
    selection: Option<&super::launch_options::AgentSelection>,
    cwd: Option<&Path>,
    timeout: Duration,
) -> Result<String, HeadlessError> {
    if selection.is_some_and(|choice| choice.agent != summarizer.kind) {
        return Err(HeadlessError::InvalidSelection(
            "agent does not match the resolved one-shot runner".to_string(),
        ));
    }
    let args = match selection {
        Some(choice) => super::launch_options::apply_selection(None, choice),
        None => super::launch_options::apply(summarizer.kind, None, None, None),
    }
    .map_err(HeadlessError::InvalidSelection)?;
    let selection = super::inject::split_extra_args(args.as_deref());
    let mut args: Vec<&str> = summarizer.spec.args.to_vec();
    args.extend(selection.iter().map(String::as_str));
    let stdout = run(
        &summarizer.bin,
        &args,
        prompt,
        summarizer.spec.prompt,
        cwd,
        timeout,
    )?;
    decode_output(summarizer.spec.output, &stdout)
}

/// Strip one agent's transport framing from stdout.
fn decode_output(shape: OutputShape, stdout: &str) -> Result<String, HeadlessError> {
    if shape == OutputShape::Plain {
        let answer = stdout.trim().to_string();
        return (!answer.is_empty())
            .then_some(answer)
            .ok_or(HeadlessError::Empty);
    }
    let mut parts = Vec::new();
    for line in stdout.lines().filter(|line| !line.trim().is_empty()) {
        let Ok(event) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if matches!(
            event.get("type").and_then(serde_json::Value::as_str),
            Some("error")
        ) {
            return Err(HeadlessError::Failed {
                code: None,
                stderr: "OpenCode returned an error event".to_string(),
            });
        }
        let text = event
            .get("part")
            .filter(|part| part.get("type").and_then(serde_json::Value::as_str) == Some("text"))
            .and_then(|part| part.get("text"))
            .and_then(serde_json::Value::as_str)
            .or_else(|| {
                (event.get("type").and_then(serde_json::Value::as_str) == Some("text"))
                    .then(|| event.get("text").and_then(serde_json::Value::as_str))
                    .flatten()
            });
        if let Some(text) = text.filter(|text| !text.trim().is_empty()) {
            parts.push(text.trim().to_string());
        }
    }
    let answer = parts.join("\n\n");
    (!answer.is_empty())
        .then_some(answer)
        .ok_or(HeadlessError::Empty)
}

/// Wait for the child, giving up after `timeout`. Returns None when the deadline passed first.
///
/// `Child` has no timed wait, so poll it. The interval is short enough to feel immediate and long enough
/// that a multi-minute run costs almost nothing to watch.
fn wait_with_deadline(
    child: &mut std::process::Child,
    timeout: Duration,
) -> Option<std::process::ExitStatus> {
    const POLL: Duration = Duration::from_millis(50);
    let deadline = std::time::Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status),
            // A child that vanished cannot be waited for; treat it as finished rather than spinning to
            // the deadline.
            Err(_) => return None,
            Ok(None) => {}
        }
        if std::time::Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(POLL.min(deadline.saturating_duration_since(std::time::Instant::now())));
    }
}

/// Kill the child and anything it started.
fn terminate(child: &mut std::process::Child) {
    #[cfg(unix)]
    {
        // Signal the negative process-group id so the agent's own helper processes go too; `Child::kill`
        // alone would leave them holding the pipes.
        unsafe {
            libc::kill(-(child.id() as i32), libc::SIGKILL);
        }
    }
    #[cfg(windows)]
    {
        // Windows has no process groups here; taskkill /T removes the tree.
        let pid = child.id().to_string();
        let _ = crate::host::command("taskkill")
            .args(["/PID", pid.as_str(), "/T", "/F"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let _ = child.kill();
    }
}

/// Keep the last `max` characters, which is where a failure reason usually is.
fn tail(text: &str, max: usize) -> String {
    let trimmed = text.trim();
    let count = trimmed.chars().count();
    if count <= max {
        return trimmed.to_string();
    }
    let kept: String = trimmed.chars().skip(count - max).collect();
    format!("…{kept}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spec_covers_the_verified_agents_and_refuses_the_rest() {
        for kind in [
            SessionKind::Claude,
            SessionKind::Codex,
            SessionKind::Opencode,
            SessionKind::Pi,
            SessionKind::Omp,
            SessionKind::Cursor,
            SessionKind::Copilot,
            SessionKind::Grok,
        ] {
            let spec = spec(kind).unwrap_or_else(|| panic!("{kind:?} should be invocable"));
            assert!(!spec.bin.is_empty() && !spec.args.is_empty(), "{kind:?}");
        }
        // Kinds with no verified one-shot mode must say so rather than be guessed at.
        assert!(spec(SessionKind::Terminal).is_none());
        assert!(spec(SessionKind::Browser).is_none());
        assert!(spec(SessionKind::Cline).is_none());
    }

    #[test]
    fn only_stdin_agents_accept_a_long_prompt() {
        // A whole transcript is far past any argument limit, so these two are the only ones that can
        // summarize a large session.
        assert!(spec(SessionKind::Claude).unwrap().accepts_long_prompt());
        assert!(spec(SessionKind::Codex).unwrap().accepts_long_prompt());
        assert!(!spec(SessionKind::Cursor).unwrap().accepts_long_prompt());
        assert!(!spec(SessionKind::Copilot).unwrap().accepts_long_prompt());
        assert!(!spec(SessionKind::Grok).unwrap().accepts_long_prompt());
        assert!(!spec(SessionKind::Opencode).unwrap().accepts_long_prompt());
        assert!(!spec(SessionKind::Pi).unwrap().accepts_long_prompt());
        assert!(!spec(SessionKind::Omp).unwrap().accepts_long_prompt());
        // Codex needs `-` to be told stdin carries the prompt; claude needs no such marker.
        assert_eq!(
            spec(SessionKind::Codex).unwrap().prompt,
            PromptDelivery::Stdin { marker: Some("-") }
        );
        assert_eq!(
            spec(SessionKind::Claude).unwrap().prompt,
            PromptDelivery::Stdin { marker: None }
        );
    }

    #[test]
    fn spec_keys_match_the_settings_and_probe_names() {
        // The key indexes configured executable paths and installation probes; the binary name is
        // separate because Cursor's command is not called `cursor`.
        assert_eq!(spec(SessionKind::Cursor).unwrap().key, "cursor");
        assert_eq!(spec(SessionKind::Cursor).unwrap().bin, "cursor-agent");
        for kind in [SessionKind::Claude, SessionKind::Codex, SessionKind::Grok] {
            let spec = spec(kind).unwrap();
            assert_eq!(spec.key, spec.bin, "{kind:?} uses one name for both");
        }
    }

    #[test]
    fn candidates_put_the_callers_own_agent_first() {
        // The user picked codex, so the summary should run on codex and bill to codex.
        let got = candidates(None, Some(SessionKind::Codex), 100);
        assert_eq!(got.first(), Some(&SessionKind::Codex));
        assert!(got.len() > 1, "the rest stay as fallbacks: {got:?}");

        // A caller whose own kind cannot summarize is simply skipped, not an error.
        let got = candidates(None, Some(SessionKind::Terminal), 100);
        assert_eq!(got, PREFERENCE.to_vec());

        // No caller at all still yields the standard order.
        assert_eq!(candidates(None, None, 100), PREFERENCE.to_vec());
    }

    #[test]
    fn candidates_honor_an_explicit_choice_exactly() {
        // `--with grok` means grok or nothing; quietly using claude instead would make the flag a lie.
        assert_eq!(
            candidates(Some(SessionKind::Grok), Some(SessionKind::Claude), 100),
            vec![SessionKind::Grok]
        );
        // Including when the named agent cannot summarize at all.
        assert!(candidates(Some(SessionKind::Cline), Some(SessionKind::Claude), 100).is_empty());
        // And when it cannot take a prompt this large.
        assert!(candidates(Some(SessionKind::Grok), None, ARG_PROMPT_LIMIT + 1).is_empty());
    }

    #[test]
    fn a_long_prompt_narrows_the_field_to_the_stdin_agents() {
        // A whole transcript cannot go in an argument, so only claude and codex remain — even for a
        // caller running something else.
        let got = candidates(None, Some(SessionKind::Cursor), ARG_PROMPT_LIMIT + 1);
        assert_eq!(got, vec![SessionKind::Claude, SessionKind::Codex]);
        assert!(
            !got.contains(&SessionKind::Cursor),
            "the caller's own agent cannot take it: {got:?}"
        );
        // Right at the limit an argument still fits.
        assert!(candidates(None, None, ARG_PROMPT_LIMIT).contains(&SessionKind::Cursor));
    }

    #[cfg(unix)]
    #[test]
    fn find_on_path_locates_an_executable_and_ignores_the_rest() {
        assert!(find_on_path("sh").is_some_and(|p| p.ends_with("sh")));
        assert!(find_on_path("vlx-definitely-not-a-command").is_none());

        // A file that exists but is not executable does not count as installed.
        let dir = std::env::temp_dir().join(format!("vlx-headless-path-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let name = "vlx-fake-agent";
        std::fs::write(dir.join(name), "#!/bin/sh\n").unwrap();
        assert!(!is_executable_file(&dir.join(name)), "0644 is not runnable");
        // A directory sharing the name is not a command either.
        std::fs::create_dir_all(dir.join("vlx-fake-dir")).unwrap();
        assert!(!is_executable_file(&dir.join("vlx-fake-dir")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn run_returns_stdout_for_both_prompt_deliveries() {
        // `cat` echoes stdin, standing in for an agent that reads its prompt there.
        let got = run(
            "cat",
            &[],
            "the whole transcript",
            PromptDelivery::Stdin { marker: None },
            None,
            Duration::from_secs(10),
        )
        .expect("stdin delivery should succeed");
        assert_eq!(got, "the whole transcript");

        // `echo` stands in for an agent that takes its prompt as an argument.
        let got = run(
            "echo",
            &[],
            "answer text",
            PromptDelivery::Arg { marker: None },
            None,
            Duration::from_secs(10),
        )
        .expect("argument delivery should succeed");
        assert_eq!(
            got, "answer text",
            "surrounding whitespace should be trimmed"
        );
    }

    #[cfg(unix)]
    #[test]
    fn run_reports_failures_without_hanging() {
        // A nonzero exit carries its stderr so the caller can say why.
        let err = run(
            "sh",
            &["-c", "echo bad things >&2; exit 3"],
            "",
            PromptDelivery::Stdin { marker: None },
            None,
            Duration::from_secs(10),
        )
        .expect_err("a nonzero exit should be an error");
        let HeadlessError::Failed { code, stderr } = err else {
            panic!("should report the exit code");
        };
        assert_eq!(code, Some(3));
        assert!(stderr.contains("bad things"), "got: {stderr}");

        // A missing binary is a start failure, not a hang.
        assert!(matches!(
            run(
                "vlx-no-such-binary-anywhere",
                &[],
                "",
                PromptDelivery::Arg { marker: None },
                None,
                Duration::from_secs(5)
            ),
            Err(HeadlessError::Start(_))
        ));

        // Success with nothing on stdout is not an answer.
        assert!(matches!(
            run(
                "true",
                &[],
                "",
                PromptDelivery::Arg { marker: None },
                None,
                Duration::from_secs(10)
            ),
            Err(HeadlessError::Empty)
        ));
    }

    #[cfg(unix)]
    #[test]
    fn run_kills_the_whole_process_group_on_timeout() {
        // The shell starts a grandchild and waits on it, exactly the shape an agent has. Killing only
        // the direct child would leave the grandchild running and its pipe open.
        let started = std::time::Instant::now();
        let err = run(
            "sh",
            &["-c", "sleep 60 & wait"],
            "",
            PromptDelivery::Arg { marker: None },
            None,
            Duration::from_millis(300),
        )
        .expect_err("it should time out");
        assert!(matches!(err, HeadlessError::Timeout(_)), "got: {err}");
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "the timeout must not wait for the child to finish on its own: {:?}",
            started.elapsed()
        );
    }

    #[cfg(unix)]
    #[test]
    fn run_bounds_output_collection_after_the_main_process_exits() {
        // A helper can inherit both pipes and outlive the CLI that launched it.
        let started = std::time::Instant::now();
        let result = run(
            "sh",
            &["-c", "sleep 2 & printf answer"],
            "",
            PromptDelivery::Stdin { marker: None },
            None,
            Duration::from_millis(100),
        );
        assert!(
            matches!(result, Err(HeadlessError::Timeout(_))),
            "got: {result:?}"
        );
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[cfg(unix)]
    #[test]
    fn run_strips_the_session_variables_from_the_child() {
        // A summarizer that inherited these could call vspawn or vrefer itself, turning one reference
        // into a recursion the caller never asked for and cannot see.
        std::env::set_var("VLX_SPAWN_URL", "http://127.0.0.1:1");
        std::env::set_var("VLX_SESSION_ID", "parent-session");
        std::env::set_var("VLX_TOKEN", "secret");
        let got = run(
            "sh",
            &["-c", "echo \"[$VLX_SPAWN_URL|$VLX_SESSION_ID|$VLX_TOKEN]\""],
            "",
            PromptDelivery::Arg { marker: None },
            None,
            Duration::from_secs(10),
        )
        .expect("the probe should run");
        assert_eq!(got, "[||]", "no session variable may reach the summarizer");
        std::env::remove_var("VLX_SPAWN_URL");
        std::env::remove_var("VLX_SESSION_ID");
        std::env::remove_var("VLX_TOKEN");
    }

    #[test]
    fn run_refuses_an_oversized_argument_prompt() {
        let huge = "x".repeat(ARG_PROMPT_LIMIT + 1);
        let err = run(
            "echo",
            &[],
            &huge,
            PromptDelivery::Arg { marker: None },
            None,
            Duration::from_secs(5),
        )
        .expect_err("it should refuse rather than let the OS reject the command line");
        assert!(
            matches!(err, HeadlessError::PromptTooLarge { .. }),
            "got: {err}"
        );
    }

    #[test]
    fn opencode_json_output_keeps_only_answer_text() {
        let stdout = concat!(
            "{\"type\":\"step_start\",\"sessionID\":\"ses_1\"}\n",
            "{\"type\":\"text\",\"part\":{\"type\":\"text\",\"text\":\"first\"}}\n",
            "{\"type\":\"text\",\"part\":{\"type\":\"text\",\"text\":\"second\"}}\n"
        );
        assert_eq!(
            decode_output(OutputShape::OpenCodeJson, stdout).unwrap(),
            "first\n\nsecond"
        );
    }

    #[cfg(unix)]
    #[test]
    fn selected_run_uses_shared_model_effort_mapping_before_the_prompt() {
        let runner = Summarizer {
            kind: SessionKind::Pi,
            spec: HeadlessSpec {
                key: "pi",
                bin: "sh",
                args: &["-c", "printf '%s\\n' \"$@\"", "probe"],
                prompt: PromptDelivery::Arg { marker: Some("-p") },
                output: OutputShape::Plain,
            },
            bin: "sh".to_string(),
        };
        let selection = super::super::launch_options::AgentSelection {
            agent: SessionKind::Pi,
            model: "provider/model".to_string(),
            effort: "high".to_string(),
        };
        let got = run_selected(
            &runner,
            "summarize this",
            Some(&selection),
            None,
            Duration::from_secs(5),
        )
        .unwrap();
        assert_eq!(
            got,
            "--model\nprovider/model\n--thinking\nhigh\n-p\nsummarize this"
        );
    }

    #[test]
    fn tail_keeps_the_end_and_marks_the_cut() {
        assert_eq!(tail("  short  ", 100), "short");
        let long = "abcdefghij".repeat(10);
        let got = tail(&long, 5);
        assert_eq!(got, "…fghij");
        // Counting characters rather than bytes keeps multi-byte text from being cut mid-character.
        assert_eq!(tail("一二三四五", 2), "…四五");
    }
}

#[cfg(test)]
use super::executable::{find_on_path, is_executable_file};

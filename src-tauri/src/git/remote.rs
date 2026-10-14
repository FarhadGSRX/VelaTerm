//! Remote synchronization and stopped-operation control for the Git panel: fetch, pull, push,
//! publishing a branch, and aborting or continuing a merge, rebase, cherry-pick, or revert that
//! stopped on conflicts.
//!
//! Network commands run headless: stdin is closed and every interactive prompt is disabled, so a
//! missing credential fails with git's own message instead of waiting on a terminal nobody sees.
//! Configured credential helpers and SSH agents still work. A hard timeout kills the whole process
//! group, because a stalled connection would otherwise hold the panel busy indefinitely.

use super::*;

/// Upper bound for one network command. Large fetches finish well within it; a hang does not.
const NETWORK_TIMEOUT: Duration = Duration::from_secs(180);
/// Upper bound for local continuation commands, which may run hooks.
const LOCAL_TIMEOUT: Duration = Duration::from_secs(120);

/// How `pull` integrates upstream commits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PullMode {
    /// Only move the branch forward; refuses when local and upstream have diverged.
    FastForward,
    /// Replay local commits on top of upstream, stashing uncommitted work around it and restoring
    /// it with its staged and unstaged parts intact.
    Rebase,
    /// Create a merge commit when the histories have diverged.
    Merge,
}

impl PullMode {
    pub fn parse(mode: &str) -> Result<Self, String> {
        match mode {
            "ff-only" => Ok(Self::FastForward),
            "rebase" => Ok(Self::Rebase),
            "merge" => Ok(Self::Merge),
            other => Err(format!("Unknown pull mode: {other}")),
        }
    }
}

/// Run one git command headless with a timeout. Returns stdout, or the most telling stderr lines
/// (credentials in URLs redacted) as the error.
fn run_headless(cwd: &str, args: &[&str], timeout: Duration) -> Result<String, String> {
    let mut command = crate::host::command("git");
    command
        .arg("-C")
        .arg(cwd)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GCM_INTERACTIVE", "Never")
        .env("SSH_ASKPASS_REQUIRE", "never")
        // Continuations and merges open no editor; the message git prepared is used as is.
        .env("GIT_EDITOR", "true")
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let name = args.first().copied().unwrap_or("command");
    let mut child = command.spawn().map_err(|e| format!("Failed to run git {name}: {e}"))?;
    let mut stdout = child.stdout.take().ok_or("Failed to capture git output")?;
    let mut stderr = child.stderr.take().ok_or("Failed to capture git output")?;
    // Drain both pipes on their own threads so a chatty command cannot block on a full pipe.
    let out_reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout.read_to_end(&mut buf);
        buf
    });
    let err_reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stderr.read_to_end(&mut buf);
        buf
    });
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() >= timeout => {
                terminate_clone_process(&mut child);
                let _ = child.wait();
                return Err(format!("git {name} timed out after {} seconds", timeout.as_secs()));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(e) => {
                terminate_clone_process(&mut child);
                return Err(format!("Failed while waiting for git {name}: {e}"));
            }
        }
    };
    let out = out_reader.join().unwrap_or_default();
    let err = err_reader.join().unwrap_or_default();
    if status.success() {
        return Ok(String::from_utf8_lossy(&out).trim().to_string());
    }
    Err(failure_message(name, &String::from_utf8_lossy(&err), &String::from_utf8_lossy(&out)))
}

/// Pick the lines of a failed command worth showing: git's `fatal:`/`error:` lines and the hints
/// around a rejected push, or the tail of the output when there are none.
fn failure_message(name: &str, stderr: &str, stdout: &str) -> String {
    let lines: Vec<String> = stderr
        .lines()
        .chain(stdout.lines())
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(redact_clone_error)
        .collect();
    let telling: Vec<&String> = lines
        .iter()
        .filter(|l| {
            l.starts_with("fatal:") || l.starts_with("error:") || l.starts_with("! [")
                || l.starts_with("CONFLICT") || l.starts_with("hint: Updates were rejected")
        })
        .collect();
    let picked: Vec<&String> = if telling.is_empty() {
        lines.iter().rev().take(6).collect::<Vec<_>>().into_iter().rev().collect()
    } else {
        telling.into_iter().take(8).collect()
    };
    if picked.is_empty() {
        format!("git {name} failed")
    } else {
        picked.iter().map(|s| s.as_str()).collect::<Vec<_>>().join("\n")
    }
}

/// The checked-out branch, or an error naming why a branch is needed.
fn current_branch(cwd: &str, action: &str) -> Result<String, String> {
    run_git(cwd, &["symbolic-ref", "--quiet", "--short", "HEAD"])
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("Check out a branch before you {action}"))
}

/// Short name of the branch's upstream, such as `origin/main`; None when none is configured.
pub(super) fn upstream(cwd: &str) -> Option<String> {
    run_git(cwd, &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"])
        .filter(|s| !s.is_empty())
}

/// Configured remote names.
pub(super) fn remotes(cwd: &str) -> Vec<String> {
    run_git(cwd, &["remote"])
        .map(|s| s.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).collect())
        .unwrap_or_default()
}

/// Remote a branch without upstream is published to, following git's own precedence:
/// `branch.<name>.pushRemote`, then `remote.pushDefault`, then `origin`, then the only remote.
fn publish_remote(cwd: &str, branch: &str) -> Result<String, String> {
    let all = remotes(cwd);
    if all.is_empty() {
        return Err("This repository has no remote to publish to".into());
    }
    let configured = run_git(cwd, &["config", "--get", &format!("branch.{branch}.pushRemote")])
        .or_else(|| run_git(cwd, &["config", "--get", "remote.pushDefault"]))
        .filter(|r| all.contains(r));
    if let Some(remote) = configured {
        return Ok(remote);
    }
    if all.iter().any(|r| r == "origin") {
        return Ok("origin".into());
    }
    if all.len() == 1 {
        return Ok(all[0].clone());
    }
    Err("Several remotes are configured. Set remote.pushDefault to choose where to publish".into())
}

/// Download objects and refs from the default remote and prune deleted remote branches.
pub fn fetch(cwd: &str) -> Result<(), String> {
    require_repo(cwd)?;
    run_headless(cwd, &["fetch", "--prune"], NETWORK_TIMEOUT).map(|_| ())
}

/// Integrate the upstream branch into the current one.
pub fn pull(cwd: &str, mode: PullMode) -> Result<(), String> {
    require_repo(cwd)?;
    current_branch(cwd, "pull")?;
    if upstream(cwd).is_none() {
        return Err("This branch has no upstream branch to pull from".into());
    }
    match mode {
        PullMode::FastForward => run_headless(cwd, &["pull", "--ff-only"], NETWORK_TIMEOUT).map(|_| ()),
        PullMode::Merge => run_headless(cwd, &["pull", "--no-rebase", "--no-edit"], NETWORK_TIMEOUT).map(|_| ()),
        PullMode::Rebase => pull_rebase(cwd),
    }
}

/// Message of the stash that carries uncommitted work across a rebase pull.
const PULL_STASH: &str = "VelaTerm: uncommitted changes before pull --rebase";

/// Identity of the newest stash entry, if any.
fn top_stash(cwd: &str) -> Option<String> {
    run_git(cwd, &["rev-parse", "--verify", "--quiet", "refs/stash"]).filter(|s| !s.is_empty())
}

/// `stash@{n}` of the entry with this identity; entries shift when other stashes are pushed.
fn stash_ref(cwd: &str, oid: &str) -> Option<String> {
    let list = run_git(cwd, &["stash", "list", "--format=%H"])?;
    list.lines().position(|l| l.trim() == oid).map(|i| format!("stash@{{{i}}}"))
}

/// Rebase pull that keeps the staged/unstaged split of uncommitted work. git's own `--autostash`
/// re-applies the stash without the index, so a partly staged file would come back fully unstaged.
/// Untracked files stay in place: rebase leaves them alone unless upstream adds the same path, and
/// then git refuses before touching anything.
fn pull_rebase(cwd: &str) -> Result<(), String> {
    let before = top_stash(cwd);
    run_headless(cwd, &["stash", "push", "--quiet", "-m", PULL_STASH], LOCAL_TIMEOUT)?;
    let stashed = top_stash(cwd).filter(|oid| Some(oid) != before.as_ref());
    let pulled = run_headless(cwd, &["pull", "--rebase"], NETWORK_TIMEOUT);
    let Some(oid) = stashed else { return pulled.map(|_| ()) };
    let kept = format!("Your uncommitted changes are kept in the stash \"{PULL_STASH}\"");
    if let Err(e) = pulled {
        // A rebase stopped on conflicts must be finished first; restoring now would mix the two.
        let stopped = run_git(cwd, &["rev-parse", "--absolute-git-dir"])
            .is_some_and(|d| operation_in(std::path::Path::new(&d)).is_some());
        if stopped {
            return Err(format!("{e}\n{kept}; restore them with git stash pop --index after the rebase."));
        }
        return match restore_stash(cwd, &oid) {
            Ok(()) => Err(e),
            Err(_) => Err(format!("{e}\n{kept}.")),
        };
    }
    restore_stash(cwd, &oid)
        .map_err(|e| format!("Pulled, but restoring your uncommitted changes failed: {e}\n{kept}."))
}

fn restore_stash(cwd: &str, oid: &str) -> Result<(), String> {
    let reference = stash_ref(cwd, oid).ok_or("The stash entry is no longer there")?;
    run_headless(cwd, &["stash", "pop", "--index", "--quiet", &reference], LOCAL_TIMEOUT).map(|_| ())
}

/// Push the current branch. Without an upstream the branch is published under the same name and
/// starts tracking it. `force` uses `--force-with-lease`, which refuses to overwrite remote commits
/// this repository has not seen.
pub fn push(cwd: &str, force: bool) -> Result<(), String> {
    require_repo(cwd)?;
    let branch = current_branch(cwd, "push")?;
    if !has_head(cwd) {
        return Err("Commit something before you push".into());
    }
    if upstream(cwd).is_some() {
        let mut args = vec!["push"];
        if force {
            args.push("--force-with-lease");
        }
        return run_headless(cwd, &args, NETWORK_TIMEOUT).map(|_| ());
    }
    let remote = publish_remote(cwd, &branch)?;
    let mut args = vec!["push", "-u"];
    if force {
        args.push("--force-with-lease");
    }
    args.push(remote.as_str());
    args.push(branch.as_str());
    run_headless(cwd, &args, NETWORK_TIMEOUT).map(|_| ())
}

/// Operation that stopped in this worktree waiting for conflicts to be resolved, read from the
/// marker files git leaves in the worktree's own git directory.
pub(super) fn operation_in(git_dir: &std::path::Path) -> Option<&'static str> {
    if git_dir.join("rebase-merge").is_dir() || git_dir.join("rebase-apply").is_dir() {
        Some("rebase")
    } else if git_dir.join("MERGE_HEAD").is_file() {
        Some("merge")
    } else if git_dir.join("CHERRY_PICK_HEAD").is_file() {
        Some("cherry-pick")
    } else if git_dir.join("REVERT_HEAD").is_file() {
        Some("revert")
    } else {
        None
    }
}

fn operation(cwd: &str) -> Result<&'static str, String> {
    let git_dir = run_git(cwd, &["rev-parse", "--absolute-git-dir"])
        .ok_or("Cannot locate the git directory")?;
    operation_in(std::path::Path::new(&git_dir))
        .ok_or_else(|| "No merge, rebase, cherry-pick, or revert is in progress".to_string())
}

/// Abort the stopped operation and return the worktree to where it started.
pub fn abort_operation(cwd: &str) -> Result<(), String> {
    require_repo(cwd)?;
    let op = operation(cwd)?;
    run_headless(cwd, &[op, "--abort"], LOCAL_TIMEOUT).map(|_| ())
}

/// Continue the stopped operation once its conflicts are resolved and staged, keeping the message
/// git prepared.
pub fn continue_operation(cwd: &str) -> Result<(), String> {
    require_repo(cwd)?;
    let op = operation(cwd)?;
    run_headless(cwd, &[op, "--continue"], LOCAL_TIMEOUT).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Repo(std::path::PathBuf);

    impl Repo {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("vlx-git-remote-{name}-{}", Uuid::new_v4()));
            std::fs::create_dir_all(&dir).unwrap();
            let repo = Self(dir);
            repo.git(&["init", "-q", "-b", "main"]);
            repo.git(&["config", "user.email", "test@example.com"]);
            repo.git(&["config", "user.name", "Test"]);
            repo.git(&["config", "commit.gpgsign", "false"]);
            repo
        }
        fn bare(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("vlx-git-remote-{name}-{}", Uuid::new_v4()));
            std::fs::create_dir_all(&dir).unwrap();
            run_git_checked(dir.to_str().unwrap(), &["init", "-q", "--bare", "-b", "main"]).unwrap();
            Self(dir)
        }
        fn cwd(&self) -> &str { self.0.to_str().unwrap() }
        fn git(&self, args: &[&str]) -> String { run_git_checked(self.cwd(), args).unwrap() }
        fn commit(&self, file: &str, text: &str) {
            std::fs::write(self.0.join(file), text).unwrap();
            self.git(&["add", "-A"]);
            self.git(&["commit", "-q", "-m", file]);
        }
    }

    impl Drop for Repo { fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); } }

    #[test]
    fn publish_then_push_and_pull_track_the_remote() {
        let remote = Repo::bare("origin");
        let local = Repo::new("local");
        local.commit("a.txt", "one\n");
        local.git(&["remote", "add", "origin", remote.cwd()]);
        assert_eq!(upstream(local.cwd()), None);

        push(local.cwd(), false).unwrap();
        assert_eq!(upstream(local.cwd()).as_deref(), Some("origin/main"));

        local.commit("b.txt", "two\n");
        assert_eq!(status(local.cwd()).ahead, 1);
        push(local.cwd(), false).unwrap();
        assert_eq!(status(local.cwd()).ahead, 0);

        // A second clone advances the remote; the first fetches, sees it behind, and fast-forwards.
        let other = Repo::new("other");
        other.git(&["remote", "add", "origin", remote.cwd()]);
        other.git(&["fetch", "-q", "origin"]);
        other.git(&["checkout", "-q", "-B", "main", "origin/main"]);
        other.commit("c.txt", "three\n");
        other.git(&["push", "-q", "origin", "main"]);

        fetch(local.cwd()).unwrap();
        assert_eq!(status(local.cwd()).behind, 1);
        pull(local.cwd(), PullMode::FastForward).unwrap();
        let st = status(local.cwd());
        assert_eq!((st.ahead, st.behind), (0, 0));
        assert!(local.0.join("c.txt").exists());
    }

    #[test]
    fn diverged_fast_forward_pull_fails_and_rebase_pull_succeeds() {
        let remote = Repo::bare("origin-div");
        let local = Repo::new("local-div");
        local.commit("a.txt", "one\n");
        local.git(&["remote", "add", "origin", remote.cwd()]);
        push(local.cwd(), false).unwrap();

        let other = Repo::new("other-div");
        other.git(&["remote", "add", "origin", remote.cwd()]);
        other.git(&["fetch", "-q", "origin"]);
        other.git(&["checkout", "-q", "-B", "main", "origin/main"]);
        other.commit("theirs.txt", "theirs\n");
        other.git(&["push", "-q", "origin", "main"]);

        local.commit("mine.txt", "mine\n");
        // Uncommitted work survives the rebase with its staged and unstaged parts apart.
        std::fs::write(local.0.join("a.txt"), "staged\n").unwrap();
        local.git(&["add", "a.txt"]);
        std::fs::write(local.0.join("a.txt"), "staged\nunstaged\n").unwrap();
        std::fs::write(local.0.join("untracked.txt"), "u\n").unwrap();
        let err = pull(local.cwd(), PullMode::FastForward).unwrap_err();
        assert!(err.contains("fatal") || err.contains("Not possible"), "{err}");
        pull(local.cwd(), PullMode::Rebase).unwrap();
        let st = status(local.cwd());
        assert_eq!((st.ahead, st.behind), (1, 0));
        assert_eq!(std::fs::read_to_string(local.0.join("a.txt")).unwrap(), "staged\nunstaged\n");
        assert_eq!(local.git(&["show", ":a.txt"]), "staged");
        assert!(local.0.join("untracked.txt").exists());
        assert_eq!(local.git(&["stash", "list"]), "");
    }

    #[test]
    fn failed_rebase_pull_restores_uncommitted_work_with_its_staging() {
        let remote = Repo::bare("origin-gone");
        let local = Repo::new("local-gone");
        local.commit("a.txt", "one\n");
        local.git(&["remote", "add", "origin", remote.cwd()]);
        push(local.cwd(), false).unwrap();
        std::fs::write(local.0.join("a.txt"), "staged\n").unwrap();
        local.git(&["add", "a.txt"]);
        std::fs::write(local.0.join("a.txt"), "staged\nmore\n").unwrap();
        // The remote disappears, so the fetch inside pull fails before any rebase starts.
        std::fs::remove_dir_all(&remote.0).unwrap();
        assert!(pull(local.cwd(), PullMode::Rebase).is_err());
        assert_eq!(local.git(&["show", ":a.txt"]), "staged");
        assert_eq!(std::fs::read_to_string(local.0.join("a.txt")).unwrap(), "staged\nmore\n");
        assert_eq!(local.git(&["stash", "list"]), "");
    }

    #[test]
    fn pushing_without_a_remote_or_branch_explains_why() {
        let local = Repo::new("noremote");
        local.commit("a.txt", "one\n");
        assert!(push(local.cwd(), false).unwrap_err().contains("no remote"));
        let head = local.git(&["rev-parse", "HEAD"]);
        local.git(&["checkout", "-q", &head]);
        assert!(push(local.cwd(), false).unwrap_err().contains("Check out a branch"));
        assert!(pull(local.cwd(), PullMode::FastForward).unwrap_err().contains("Check out a branch"));
    }

    #[test]
    fn stopped_merge_is_reported_and_can_be_aborted_or_continued() {
        let repo = Repo::new("merge-op");
        repo.commit("a.txt", "base\n");
        repo.git(&["checkout", "-q", "-b", "side"]);
        repo.commit("a.txt", "side\n");
        repo.git(&["checkout", "-q", "main"]);
        repo.commit("a.txt", "main\n");
        assert!(run_git_checked(repo.cwd(), &["merge", "side"]).is_err());
        assert_eq!(status(repo.cwd()).operation.as_deref(), Some("merge"));

        abort_operation(repo.cwd()).unwrap();
        assert_eq!(status(repo.cwd()).operation, None);
        assert_eq!(std::fs::read_to_string(repo.0.join("a.txt")).unwrap(), "main\n");

        assert!(run_git_checked(repo.cwd(), &["merge", "side"]).is_err());
        std::fs::write(repo.0.join("a.txt"), "resolved\n").unwrap();
        repo.git(&["add", "a.txt"]);
        continue_operation(repo.cwd()).unwrap();
        assert_eq!(status(repo.cwd()).operation, None);
        assert_eq!(repo.git(&["rev-list", "--count", "HEAD"]), "4");
        assert!(abort_operation(repo.cwd()).unwrap_err().contains("No merge"));
    }

    #[test]
    fn failure_message_keeps_telling_lines_and_redacts_credentials() {
        let msg = failure_message(
            "push",
            "To https://user:secret@example.com/r.git\n ! [rejected]        main -> main (fetch first)\nerror: failed to push some refs to 'https://user:secret@example.com/r.git'\nhint: Updates were rejected because the remote contains work\n",
            "",
        );
        assert!(msg.contains("! [rejected]"));
        assert!(msg.contains("error: failed to push"));
        assert!(!msg.contains("secret"));
        assert_eq!(failure_message("fetch", "", ""), "git fetch failed");
    }

    #[test]
    fn pull_modes_parse() {
        assert_eq!(PullMode::parse("ff-only").unwrap(), PullMode::FastForward);
        assert_eq!(PullMode::parse("rebase").unwrap(), PullMode::Rebase);
        assert_eq!(PullMode::parse("merge").unwrap(), PullMode::Merge);
        assert!(PullMode::parse("octopus").is_err());
    }
}

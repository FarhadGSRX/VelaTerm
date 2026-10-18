# VelaTerm Rust Backend Map (`src-tauri/`)

Surveyed at commit d098f5a, 58 tracked files, ~37.3k lines. Read-only analysis; every non-obvious
claim is cited `path:line`. Line counts are approximate (from `wc -l` at survey time).

## 1. Module map

Declared in `src/lib.rs:5-39`. `#[cfg(feature = "gui")]` gates are noted; everything else compiles
into both the desktop app and the headless `vela-server` (`--no-default-features`).

- **`agent/`** (agent/mod.rs:6-25, 15 submodules) — detection, launch-argument injection, hook
  server, resume/transcript/usage reading, and per-CLI hook installers for every supported agent kind.
  Always compiled (headless `--serve` still needs to spawn/track agents). See §4.
- **`appimage.rs`** — strips AppImage's `AppRun` environment rewrites (`PYTHONHOME`, `LD_LIBRARY_PATH`,
  etc., appimage.rs:3-9) from child processes on Linux so spawned shells/agents don't inherit a
  bundle-scoped environment.
- **`browser.rs`** (lib.rs:10, gui-only) — built-in browser tabs as native child WebViews mounted via
  `Window::add_child`; explicitly documents that `browser-*` window labels sit outside the `windows:["main"]`
  Tauri capability so an embedded page cannot call Tauri commands (browser.rs:1-9).
  Registered as 9 direct Tauri commands in the invoke handler (`browser_open`..`browser_close`).
- **`command_core.rs`** — transport-independent orchestration layer. See §2.
- **`commands.rs`** (lib.rs:14, gui-only) — the slim Tauri `#[tauri::command]` surface. See §2.
- **`db/`** — `mod.rs` (connection/PRAGMA/migration), `schema.rs` (DDL), `repo.rs` (2657 lines of
  query functions). See §6.
- **`files.rs`** — lazy, one-level-at-a-time directory listing/preview for the Files tab
  (files.rs:1-2), plus pasted/doc image persistence and stale-temp-image sweeping.
- **`git.rs`** (2216 lines) — shells out to the system `git` CLI (no libgit2, git.rs:1) for status,
  diffs, worktrees, branches, and merge preview/apply.
- **`gitea.rs`** — Gitea PR "landing" provider: token storage (keyring, falling back to plaintext
  `app_settings`), base-URL persistence, and origin-vs-configured-host provider detection
  (gitea.rs:1-9).
- **`host.rs`** — `AppCtx` abstraction (`Tauri` vs `Headless` variants) decoupling `pty/`, `agent/`,
  `web/` from `tauri::AppHandle` so the same code drives both the desktop app and `--serve`
  (host.rs:1-9).
- **`models.rs`** — `SessionKind` enum (the 15 session/agent types, see §4) and other camelCase
  persistent DTOs shared with the frontend.
- **`notify_native.rs`** (gui+macOS only) / **`notify_win.rs`** (gui+Windows only) — native
  click-through notifications (UNUserNotificationCenter / WinRT toasts) because the official Tauri
  notification plugin has no desktop click callback (notify_native.rs:1-8, notify_win.rs:1-5).
- **`procstat.rs`** — CPU/memory sampling of a PID's full descendant tree (agent process + its
  children) via `sysinfo`, replacing a prior `ps -axo` subprocess poll (procstat.rs:1-8).
- **`pty/`** — PTY subsystem. See §3.
- **`search/`** — FTS5 (trigram) full-text search across transcripts and recordings, including
  archived sessions (search/mod.rs:1-6). See §6.
- **`server_supply.rs`** (gui-only) — downloads/verifies (minisign + SHA-256) the `vela-server`
  binary from R2 for SSH remote provisioning.
- **`ssh_remote.rs`** / **`ssh_russh.rs`** (gui-only) — client-side SSH orchestration: connect,
  detect remote OS, provision `vela-server`, start `--serve --local-http`, forward via `ssh -L`
  (OpenSSH) or an in-process `russh` transport (Windows-preferred, ssh_russh.rs:1-7), open an
  auto-login client window.
- **`web/`** — the embedded HTTP/WS remote-access server. See §5.

`build.rs` and `main.rs` are covered in §7.

## 2. Tauri command surface

Two-tier design, documented at command_core.rs:1-14 and web/dispatch.rs:1-7:

- **`command_core.rs`** holds "transport-independent orchestration" for anything stateful,
  side-effecting, or multi-step (session/project/group CRUD, app settings, search, git worktrees,
  gitea, SSH/URL host bookkeeping, web-server lifecycle). Each function takes `&AppCtx` so it works
  identically from desktop or a browser/remote client. The doc comment explicitly calls out that
  centralizing here fixed real desktop/browser behavior drift bugs (delete_node recording cleanup,
  create_session error handling) (command_core.rs:8-10).
- **`web/dispatch.rs::dispatch`** (web/dispatch.rs:162) is the single `match cmd { "name" => ... }`
  router — **91 match arms** (counted directly) — shared by three callers:
  1. Tauri's `commands::desktop_call` (commands.rs:87-105), which wraps every desktop `invoke()` in
     `spawn_blocking` and calls `dispatch` with `CallOrigin::Local`.
  2. The WebSocket bridge (`web/ws.rs`) for browser/remote clients.
  3. Headless `--serve` (no GUI at all).

  Dispatch gates two hardcoded blocklists **before** touching arguments (web/dispatch.rs:173-176):
  `MANAGEMENT_CMDS` (7 cmds: web server start/stop/status, pairing create, device list/revoke,
  network interfaces — dispatch.rs:48-56) and `SECRET_CMDS` (5 cmds: gitea config, URL/SSH host
  password/record/forget — dispatch.rs:72-78) are rejected outright for `CallOrigin::Remote`. Comment
  at dispatch.rs:1032-1073 notes there are canary tests (`EXPECTED_GATED`/`EXPECTED_SECRET`) whose
  purpose is specifically to fail loudly if someone shrinks either list and silently un-gates a
  command for remote clients.
- **`commands.rs`** (lib.rs:823-871, `generate_handler!`) registers ~30 Tauri commands directly,
  reserved for things `desktop_call` can't route generically: PTY hot-path (`pty_spawn/write/resize/kill`,
  spawn needs a `tauri::ipc::Channel`), binary/persistence channels (recording read, pasted/doc
  image save, Git Bash download), main-thread-only windows/devtools, desktop-only SSH connect flow,
  desktop-only browser tabs, and macOS-only native notify commands.

So: **desktop_call + dispatch's 91 arms** is the bulk of the command surface; **~30 direct Tauri
commands** cover the exceptions. Adding a new data command to `dispatch` automatically reaches
desktop, browser, and remote clients for free (comment at lib.rs:824-825).

## 3. PTY / session lifecycle

`pty/mod.rs` defines two enums shared with the frontend as events: `AgentState` (Working/Asking/Waiting,
pty/mod.rs:14-18) and `StatusSignal` (pty/mod.rs:30-78) — a tagged union covering both authoritative
hook-derived state and legacy activity-based fallback signals (busy/bell/title/OSC-9-777 notify).

**Spawn** (`PtyManager::spawn`, pty/manager.rs:208): a session ID is the dedup key.
- If the session already exists in the map, the call **attaches** instead of relaunching: registers a
  new `OutputSink` subscriber, nudges a SIGWINCH-equivalent resize to force a full TUI redraw (comment
  explains replay may not contain the alternate screen's current frame — manager.rs:249-251), replays
  the last cached `StatusSignal` per kind (marked `silent` so it doesn't re-notify), and returns
  `attached: true` so the frontend skips re-running `initCmd` (manager.rs:213-272).
- If not, it takes a `spawning` reservation (`SpawnSlot`, manager.rs:111-133) so concurrent
  launch races from multiple clients resolve to one winner; losers poll `SPAWN_WAIT_POLL` (50ms) up
  to `SPAWN_WAIT_TIMEOUT` (3s) then either attach to the winner or error for retry
  (manager.rs:276-290, 85-87).
- Opens the PTY via `portable-pty::native_pty_system()` (manager.rs:296-303), resolves/heals the shell
  path (falls back to a discovered shell if the persisted one no longer exists, manager.rs:306-317),
  and special-cases `wsl://<distro>` shell strings into `wsl.exe --distribution <name>` (Windows-only,
  terminal-kind-only, manager.rs:319-333).
- For agent session kinds, launch arguments/injected hook config come from `agent::inject` (imported
  at manager.rs:15) — see §4.

**Output streaming**: each PTY has a dedicated reader thread that pushes chunks into an
`OutputCoalescer` (session.rs:358-419) — a 12ms coalescing window (`COALESCE_WINDOW`) or 64KB cap
(`COALESCE_MAX_BYTES`) before flushing, to avoid one WebSocket/IPC message per byte burst
(session.rs:341-345). Flushed bytes go to `OutputStream::ingest` (session.rs:17ff → around
session.rs "ingest"), which appends to a capped `ScrollbackBuffer` (512KB replay cap, `SCROLLBACK_CAP`
manager.rs:70), feeds a `ModeTracker` that remembers alternate-screen/mouse/paste mode state for
prelude replay to late subscribers, then fans out to every registered `Subscriber` sink
(session.rs:305-309, mirroring desktop IPC and WebSocket clients through the same interface). A
session can be optionally recorded to disk (`Recorder`, session.rs:17-27) up to a 50MB cap
(`RECORDING_CAP`, manager.rs:73) if the user enabled the `vlx-record-sessions` setting.

**Resume/survival**: a session survives detach — `detach` (manager.rs:1283) just removes one
subscriber and, if it was the size owner, releases size ownership (`resize_ownership_arbitration`
test at manager.rs:1922 documents the arbitration rule: `resize_allowed`, manager.rs:1358). The PTY
process itself keeps running until `kill`/`kill_all` (manager.rs:1260-1282) or process exit; a
background reader thread removes the entry from `sessions` at EOF to release descriptors promptly
(struct doc, manager.rs:97-99). Cross-restart resume (reopening a persisted session and reattaching
its underlying **agent CLI conversation**, not the OS process) is handled separately by
`agent::resume` (§4) using each agent's on-disk session/rollout files, keyed by `agent_session_id` in
SQLite (db/schema.rs "sessions.agent_session_id").

**Status detection** is dual-mode: a background `spawn_status_monitor` (manager.rs:1489) polls output
activity (`OutputScanner`, pty/monitor.rs) to infer Working/Asking via a hysteresis window (3 active
200ms polls to enter working, 4 idle to leave — `WORK_ENTER_TICKS`/`IDLE_LEAVE_TICKS`, manager.rs:49-58),
while officially-hooked agents (Claude/Codex/Kimi/etc.) instead POST structured state to the local
`HookServer` (§4), which is authoritative and disables the activity-fallback override once seen
(`StatusSignal::State.authoritative`, pty/mod.rs:37-41). A separate `spawn_idle_heal` thread
(manager.rs:1539) force-heals a stuck "working" state to "waiting" after 6s of complete silence
(`QUIET_HEAL`, manager.rs:65) — guards against a missed Stop hook.

## 4. Agent integration

`SessionKind` (models.rs:19-44) lists 15 kinds: `Terminal`, `Claude`, `Codex`, `Opencode`,
`Copilot`, `Cursor`, `Antigravity`, `Cline`, `Pi`, `Kimi`, `Kiro`, `Grok`, `Zoo`, `Crush`, `Browser`.
`agent/mod.rs:6-25` has 15 submodules but **no `claude.rs` or `codex.rs`** — Claude and Codex, being
the two "official hooks" agents with first-class support, are built directly in `agent/inject.rs`
(`build_claude_settings`, inject.rs:107; `build_codex_notify`/`build_codex_hooks`, inject.rs:145,173).
The other per-agent files (`antigravity.rs`, `cline.rs`, `copilot.rs`, `crush.rs`, `cursor.rs`,
`grok.rs`, `kimi.rs`, `kiro.rs`, `opencode.rs`, `pi.rs`) each implement roughly the same shape for
their CLI: `<agent>_home()` (config dir resolution), a hook-script/config-block template, an idempotent
merge-into-existing-config `install()`, and (for some) `audit_install()` that logs install
success/failure **without ever logging paths, tokens, or session content** (explicit comment,
kimi.rs:22). Example: `kimi.rs` writes a static POSIX shell script to
`~/.kimi-code/vlx-term/hook.sh` (kimi.rs:14-19) that no-ops unless `VLX_SPAWN_URL`/`VLX_SESSION_ID`/
`VLX_TOKEN` env vars are present (i.e., only when launched by VelaTerm), then curls the local hook
endpoint.

**Launch injection** (`agent/inject.rs`, 2866 lines, the largest file in the crate):
- `HookEndpoint { port, token }` (agent/server.rs:63-66) is embedded into whatever the agent's hook
  mechanism accepts: a `--settings` JSON blob for Claude (`build_claude_settings_at`, inject.rs:112),
  a `notify`/hooks TOML snippet for Codex (inject.rs:145-211), or an `OPENCODE_CONFIG_CONTENT` env var
  for OpenCode (inject.rs:212-227, since OpenCode has no CLI flag for this).
- `permission_flag()` (inject.rs:313) maps VelaTerm's generic staged/skip permission mode to each
  agent's actual CLI flag.
- Resume uses `valid_resume_id()` (inject.rs:377) plus each agent's native resume flag, built from the
  `agent_session_id` captured after first launch.
- Not-found handling: `report_not_found`/`not_found_message` (inject.rs:403-432) produce the
  `AgentMissing` `StatusSignal` (pty/mod.rs:67-70) surfaced by `agent/install.rs`'s `InstallRecipe`
  (install.rs:15-31) — OS-specific install commands/doc links, single source of truth so "install
  guidance" changes touch one module (install.rs:1-9).

**Status callback transport** (`agent/server.rs::HookServer`): binds `127.0.0.1:0` (random port) plus
a per-process random UUID token (server.rs:76-90); `/hook/<sid>?t=<token>&e=<event>` both identifies
the session and **rejects forged callbacks from any other local process** lacking the token (module
doc, server.rs:1-5). It returns 200 immediately so the agent is never blocked on delivery, and maps
each event to a `StatusSignal` emitted as `pty://status/{sid}`. The same server also parses `/spawn`
(child-session requests from a Claude `/vspawn` skill, server.rs:19-32) and `/view` (open-file/URL
requests, server.rs:35-52) POSTed by thin CLI shims (`agent/cli_client.rs`, installed on PATH by
`agent/spawn_cli.rs` as `vela --spawn`/`vela --view`).

**Resume/transcript/usage** (`agent/resume.rs` 948 lines, `agent/transcript.rs` 1485 lines,
`agent/usage.rs` 725 lines, `agent/export.rs` 692 lines): read each agent's own on-disk session
files directly (Claude/Codex JSONL rollouts, Pi's format, etc.) — `resume.rs` locates/waits for a
newly-created rollout file after spawn to capture its ID (`spawn_codex_capture`, resume.rs:358;
`spawn_pi_capture`, resume.rs:407); `transcript.rs` extracts context-window usage and per-turn stats,
including a **live** Codex rate-limit query path that talks to a running `codex app-server` process
(`live_codex_rate_limits`, transcript.rs:505); `export.rs` converts a transcript to Markdown per-agent
(`claude_events`, export.rs:255; `codex_events`, export.rs:410).

## 5. The web/remote server (`web/`)

Purpose: serve the SPA and bridge PTY/commands to browser and mobile clients, either as a local Tauri
sidecar or over LAN for true remote access.

**Transport** (`web/mod.rs`): embeds `../dist` via `rust_embed` in release, reads from disk in debug
(struct `Assets`, mod.rs:42, confirming the "Already established" fact); `build_router` (mod.rs:480)
wires an axum SPA + `/ws` WebSocket route. Modes (`ServeMode`, referenced mod.rs:696): loopback
plaintext (desktop's own local sidecar), LAN TLS (self-signed cert via `tls.rs`), and LAN plaintext
(**dev-only**, gated below).

**`is_production_identifier`** (mod.rs:137-139): `identifier.ends_with(".release") ||
identifier.ends_with(".server")`. It gates exactly one thing in each of its two call sites, both of
which forbid **plaintext LAN HTTP** (0.0.0.0 binding without TLS) on a production build:
- `command_core.rs:747` (`lan_http_guard`, called from the GUI's `web_server_start` command) —
  returns an error telling the user to use TLS + fingerprint pinning instead.
- `lib.rs:1163-1171` (headless `--serve --lan-http` startup path) — same rejection.

  Loopback plaintext and LAN **TLS** are unaffected by this check either way — it purely blocks the
  plaintext+LAN combination in a `.release`/`.server`-suffixed build. The repo's published
  `tauri.conf.json:5` identifier is `io.vlinx.vlxterm` (no suffix, i.e. **not** production per this
  function), confirming the "Already established" claim: an out-of-the-box build of this public
  checkout is treated as dev and CAN serve `--lan-http` in plaintext. Unit tests at mod.rs:896-908
  pin the exact suffix behavior.

**Auth** (`web/auth.rs`): Argon2id password hashing, PHC string only, no plaintext ever persisted
(module doc, auth.rs:2-5). Successful login returns a random in-memory bearer token (`Authorization:
Bearer` or `?token=` on WS) — cookies were deliberately removed (auth.rs:8-9, cross-window credential
overwrite bug). Separately, a long-lived **pairing token** embedded in QR/pairing links persists to
disk (`access_store.rs`) so restarts don't invalidate outstanding links; device self-reported
ID/name build a revocable registry, capped (auth.rs:50ish) against a malicious paired device
flooding it.

**E2EE** (`web/e2ee.rs`): NaCl box (X25519 + XSalsa20-Poly1305), wire-compatible with tweetnacl-js
(e2ee.rs:3). Server persists a long-term keypair (`vlx-e2ee-key.b64` in the data dir); the public key
rides in the pairing-link URL fragment. Handshake (e2ee.rs:11-15): client sends plaintext
`e2ee_hello`, server replies plaintext `e2ee_ready`, client sends encrypted `e2ee_auth` (device
token + optional password), server replies encrypted `e2ee_authenticated` or an encrypted error.
After that, `ws.rs` transparently encrypts/decrypts every application frame through the derived
`Cipher`.

**Rate limiting** (`web/rate_limit.rs`): explicitly framed as a DoS-amplification defense, since
unauthenticated login/E2EE-auth both cost the server a memory-hard Argon2 hash (rate_limit.rs:3-5).
Two independent brakes: `LoginRateLimiter` (5 failures / 60s fixed window per IP, atomic
reserve-then-record via an RAII `AttemptGuard` so concurrent requests can't all slip under the limit
before any records failure — rate_limit.rs:1-16) and a process-wide `VERIFY_SEMAPHORE` capping
concurrent Argon2 verifications to 2 (rate_limit.rs:27-30), independent of instance count.

**Tunnel** (`web/tunnel.rs`): a loopback-only TLS-terminating proxy that exists solely because
wry 0.55's WKWebView on macOS can't handle self-signed cert challenges — the desktop remote-connection
window navigates to `http://127.0.0.1:<tunnel-port>` and the tunnel does the real TLS to the remote
host, accepting the self-signed cert on the app's behalf (module doc, tunnel.rs:1-13). Explicitly
scoped: binds only 127.0.0.1, and the permissive cert verifier applies only to this
explicit-user-initiated tunnel path (tunnel.rs:11-13).

**Same-port HTTP→HTTPS sniff** (`web/sniff.rs`): peeks the first TLS-record byte before handshake to
distinguish a bare `host:port` HTTP request from real TLS on the one exposed port, and 301-redirects
plaintext requests to `https://` (sniff.rs:1-6).

**Command gating**: covered in §2 — `CallOrigin::Remote` (mapped from `ServeMode` at
dispatch.rs:34-40: only `LoopbackHttp` counts as `Local`) is blocked from `MANAGEMENT_CMDS` and
`SECRET_CMDS`. Framed explicitly as: a remote paired device is trusted with a shell (i.e. arbitrary
code execution is already in-scope for a paired device — that's the product), but not with rotating
its own admission credentials or reading stored secrets (dispatch.rs:25-27).

**Security model summary**: threat model = an already-paired remote device is fully trusted for shell
access (by design — that's the app's purpose) but sandboxed away from the credential/administration
plane. The actual gate against *unpaired* access is Argon2id password + rate limiting + (for
browser/E2EE clients) the NaCl-box pairing handshake; TLS (or the loopback/tunnel bypass) protects
the wire; `is_production_identifier` exists purely to stop a shipped build from ever offering the
plaintext-LAN escape hatch meant for local device testing.

## 6. Persistence

SQLite via `rusqlite` (bundled), file `vlx-term.db` in the Tauri `app_data_dir()` (identifier-scoped,
per "Already established"). Connection setup: WAL journal mode + `PRAGMA foreign_keys = ON`
(db/mod.rs:36), `busy_timeout` set first deliberately (comment at db/mod.rs:30-31, ordering matters).

**Schema** (`db/schema.rs`): `projects` → `groups` (self-referential parent, recursive tree) →
`sessions` (parent_session_id for nested/child sessions), all with soft-delete `deleted_at` tombstones
so a project/group holding *archived* sessions isn't actually removed (schema.rs:4-10) — tombstones
are only physically cleared once their last archived session is restored/deleted. Plus
`app_settings` (generic key/value, shared Tauri+Electron preference store, schema.rs:72-78),
`ssh_hosts`/`url_hosts`/`url_host_keys` (connection history + TLS trust-on-first-use anchors;
passwords live in the OS keyring, never this table — schema.rs:80-101), and
`search_index_state` (FTS freshness bookkeeping, schema.rs:110-118).

**Migrations**: no version table — `migrate()` (db/mod.rs:73) is one long idempotent function that
`ALTER TABLE ... ADD COLUMN` for each column, each guarded by `column_exists()` (db/mod.rs:196) so
reruns are no-ops. Comment calls this "M2's simple migration strategy" (schema.rs:3). Ownership:
whoever adds a session/project/group field adds one guarded `ALTER` in `db/mod.rs::migrate` — there's
no separate migration-file mechanism to learn.

**`db/repo.rs`** (2657 lines) is the raw SQL query layer — one function per operation
(`import_project`, `create_session_full`, `persist_session`, `set_agent_session_id`,
`claim_codex_session_id`, app-settings CRUD, ssh/url host CRUD, etc.), called by `command_core.rs`
under `ctx.db().conn.lock()`. It does not itself decide desktop-vs-remote behavior — that's
`command_core`'s job (§2).

**Search** (`search/`): SQLite FTS5 with the `trigram` tokenizer (substring + CJK matching,
diacritic-insensitive, search/mod.rs:3-4), created separately from `SCHEMA` with a graceful
degrade-without-search fallback if the SQLite build lacks trigram support (schema.rs "SESSION_FTS_DDL"
comment, db/schema.rs bottom). Two extraction tracks (`search/index.rs:3-8`): parsed-transcript track
for Claude/Codex sessions with a captured `agent_session_id`, ANSI-stripped-recording track
(`search/ansi.rs`, a dependency-free terminal-state-machine) for everything else. Indexing runs only
at session-stop or just-before-search — PTY reader threads never touch SQLite directly
(index.rs:6-7).

## 7. Build/packaging reality

**Feature flags** (Cargo.toml:14-30): `gui` (default) pulls in Tauri/wry/WebKit + all desktop-only
plugins (opener, dialog, notification, clipboard, updater, process, single-instance) plus
`minisign-verify` and `russh`/`russh-sftp` (GUI-only SSH client). `--no-default-features` drops all
of that and builds `vela-server`: a headless binary retaining `--serve`/`--version`/CLI shim paths.
Shared pty/agent/web/db/command_core code has zero Tauri dependency (Cargo.toml:15-16), confirmed by
the module list in §1 — only `browser.rs`, `commands.rs`, `server_supply.rs`, `ssh_remote.rs`,
`ssh_russh.rs`, and the two platform notify modules are `#[cfg(feature = "gui")]`.

**`main.rs`** dispatches by argv before doing anything else: `--version`/`-V` (main.rs:14), a hidden
`--vela-help`, hidden hook/notify forwarders (`--notify`, `--notify-env`, `--codex-hook`,
`--grok-hook`, main.rs:26-36), hidden CLI-shim subcommands (`--spawn`, `--view`, used by the `vela`
PATH shim installed by `agent/spawn_cli.rs`), then `--serve` for headless mode, falling through to
the GUI (`#[cfg(feature = "gui")]`) or a headless usage-and-exit message otherwise (main.rs:87-93).
Also raises the process's open-file soft limit to 65536 (`raise_fd_limit`, main.rs:96-107, macOS's
default 256 is too low for Node-based agents) and, on non-Windows GUI launches, re-derives `PATH`
from the user's login shell (main.rs:126-177) because Dock/Finder launches inherit launchd's minimal
PATH.

**`build.rs`**: injects `VLX_GIT_COMMIT` (env override → `git rev-parse HEAD` → `"unknown"`,
build.rs:14-27) used for `--version` and SSH remote version-pinning (comment says commit, not semver,
is the reliable match key — build.rs:5-6). Only calls `tauri_build::build()` when
`CARGO_FEATURE_GUI` is set (build.rs:39-45), i.e. the headless server build skips
`generate_context!`/frontend-dist requirements entirely — **this means `vela-server` can build without
`../dist` existing**, only the full `gui` build needs the frontend already built.

**Windows bundling / gitbash claim — CONFIRMED, with more precision than "Already established"
stated**: `tauri.windows.conf.json` bundles `resources/gitbash/**/*`. On this checkout,
`src-tauri/resources/` contains only two files (`vlx-opencode-notify.js`, `vlx-pi-notify.ts`) — **no
`gitbash/` subdirectory exists at all**, not just an unpopulated one. `scripts/gitbash/` exists but
holds only `.gitattributes` and `zz-vlx-term.sh`; there is no `fetch-gitbash.ps1` anywhere in the
tree (checked via full-tree glob). `agent/gitbash.rs:1-9`'s own doc comment says
`fetch-gitbash.ps1` trims PortableGit into a ~25MB bundle under `resources/gitbash/` **at build
time** — that script is simply absent from this public fork. Net effect: a clean checkout (this one
included) can run `cargo build`/`cargo check` fine (gitbash.rs code-paths are conditional on the
resource existing at runtime, not compile time), but **`tauri build` for a Windows bundle will fail
or silently ship without bundled Git Bash** unless someone reconstructs or vendors that resource tree
first. This is a packaging-time gap, not a compile-time one.

**What an outside contributor can/can't do**: can `cargo check`/`cargo build --no-default-features`
(headless) with nothing extra. Can `cargo build` (full `gui`) once `../dist` exists (i.e. after a
frontend build) — Tauri's `generate_context!` needs it. Cannot produce a distributable Windows
installer/bundle without first obtaining the Git Bash resource tree by some other means (the fetch
script that would normally produce it isn't in the repo).

---

## Summary of what would need to change for a user-supplied theme file

**Nothing in Rust, almost certainly.** The only "theme" awareness in `src-tauri/` is:
1. An opaque `app_settings` key `vlx-theme` (db/schema.rs:73, db/mod.rs:291-305) — stored/round-tripped
   as an arbitrary TEXT value with no parsing, enum, or validation on the Rust side at all.
2. A `light`/`dark` **brightness string** (not a palette) threaded through PTY spawn
   (pty/manager.rs:227-228, commands.rs:135-136) purely to work around ConPTY not answering OSC 11 —
   it's written verbatim into Claude's `--settings` JSON on Windows only (manager.rs:554-572) and is a
   two-value enum, not a color scheme.

Actual color/palette definitions live entirely in the frontend (CSS/JS), which this agent did not
survey (out of scope — see `frontend-map.md`). Since `app_settings` is already a generic opaque
string store, a user-supplied theme file's *storage* path requires no backend change; only its
*parsing/application* (frontend-only) and possibly a *file-picker* Tauri command (if the file must be
read from an arbitrary disk path outside the existing settings channel) would need new code — and
that would be a small addition to `commands.rs`/`dispatch.rs`, not a redesign.

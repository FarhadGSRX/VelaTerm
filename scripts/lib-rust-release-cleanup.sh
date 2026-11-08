#!/usr/bin/env bash
# Opt-in cleanup for a dedicated release checkout; never clean a shared development target.
release_clean_rust() {
  [ "${VLX_RELEASE_CLEAN_RUST:-0}" = 1 ] || return 0
  local owned native target manifest
  owned="$(cd "${VLX_RELEASE_TASK_ROOT:?Set the dedicated release checkout}" && pwd -P)"
  [ "$owned" = "$(cd "$ROOT" && pwd -P)" ] || { echo "Release cleanup ownership mismatch: $ROOT" >&2; return 1; }
  native="$ROOT"
  case "$(uname -s)" in MINGW*|MSYS*|CYGWIN*) native="$(cygpath -m "$ROOT")" ;; esac
  manifest="$native/src-tauri/Cargo.toml"
  target="$(cargo metadata --manifest-path "$manifest" --offline --no-deps --format-version 1 | node -e '
    const fs = require("fs"), path = require("path");
    const actual = JSON.parse(fs.readFileSync(0, "utf8")).target_directory;
    const expected = path.resolve(process.argv[1], "src-tauri/target");
    const normalize = p => process.platform === "win32" ? path.resolve(p).toLowerCase() : path.resolve(p);
    if (normalize(actual) !== normalize(expected)) throw new Error("Refusing cleanup of an unexpected Cargo target: " + actual);
    process.stdout.write(actual);' "$native")" || return 1
  [ -d "$ROOT/src-tauri/target" ] || return 0
  cargo clean --manifest-path "$manifest" || return 1
  [ ! -d "$ROOT/src-tauri/target" ] || { echo "Rust cleanup left artifacts: $target" >&2; return 1; }
  printf 'Rust release artifacts removed: %s\n' "$target"
}

# bash 3.2 (macOS /bin/bash) hands an EXIT trap $?=0 when `set -u` aborts the script on an unset variable, and
# the script then exits 0, so a caller's `|| die` never fires. In v0.2.12 this let release.sh stop before deploying
# while publish.sh packaged the stale app left in ~/Soft. Scripts that install the trap call release_mark_done
# right before every successful exit; reaching the trap without it counts as a failure.
__RELEASE_DONE=0
release_mark_done() { __RELEASE_DONE=1; }

# Preserve the original failure while attempting cleanup of partially compiled artifacts.
release_cleanup_on_failure() {
  local rc="$1"
  if [ "$rc" -eq 0 ] && [ "$__RELEASE_DONE" -ne 1 ]; then
    echo "Script stopped before finishing (for example on an unset variable); treating it as a failure" >&2
    rc=1
  fi
  if [ "$rc" -ne 0 ]; then
    release_clean_rust || true
    exit "$rc"
  fi
  return 0
}

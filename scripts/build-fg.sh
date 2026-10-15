#!/usr/bin/env bash
#
# build-fg.sh — Build the "VelaTerm fg" edition: the one command for fg's build on every machine.
#
# tauri.fg.conf.json gives the edition its own product name, identifier (and so its own data dir, lock and
# credentials), binary name and a disabled updater. It overlays tauri.conf.json rather than editing it, because
# upstream rewrites tauri.conf.json in every release and an edit there would conflict on every sync.
#
# VLX_USER_INTEGRATIONS=on (the default) makes this edition manage the user-global vela command and the
# ~/.claude / ~/.codex skills. Set it to off when the official VelaTerm also runs on the machine, so the two
# editions do not overwrite each other's copies.
#
# Usage:  scripts/build-fg.sh           Windows: NSIS installer   macOS: VelaTerm fg.app
#         scripts/build-fg.sh --swap    Windows: bare velaterm-fg.exe, to rename-and-copy over a running install
#                                       (an installer closes the app, and with it any session running inside)
#
# Output: src-tauri/target/release/bundle/nsis/VelaTerm fg_<version>_x64-setup.exe   (Windows)
#         src-tauri/target/release/velaterm-fg.exe                                  (Windows, --swap)
#         src-tauri/target/release/bundle/macos/VelaTerm fg.app                      (macOS)
#
set -euo pipefail

cd "$(dirname "$0")/.."

swap=false
if [ "${1:-}" = "--swap" ]; then swap=true; shift; fi

case "$(uname -s)" in
  MINGW*|MSYS*|CYGWIN*)
    if [ ! -f src-tauri/resources/gitbash/usr/bin/bash.exe ]; then
      echo "✗ src-tauri/resources/gitbash is empty; copy it from an installed VelaTerm's resources/gitbash first" >&2
      exit 1
    fi
    if $swap; then bundle=(--no-bundle); else bundle=(--bundles nsis); fi ;;
  Darwin)
    # The .app is also what the macOS swap stages, so --swap changes nothing here.
    bundle=(--bundles app) ;;
  *)
    echo "✗ build-fg.sh supports Windows and macOS only" >&2
    exit 1 ;;
esac

echo "==> building VelaTerm fg from $(git rev-parse --abbrev-ref HEAD) @ $(git rev-parse --short HEAD)"

if command -v pnpm >/dev/null; then pnpm=(pnpm); else pnpm=(corepack pnpm); fi
[ -d node_modules ] || { echo "==> node_modules is missing, running pnpm install first"; "${pnpm[@]}" install; }

export VLX_USER_INTEGRATIONS="${VLX_USER_INTEGRATIONS:-on}"
exec "${pnpm[@]}" tauri build --config src-tauri/tauri.fg.conf.json "${bundle[@]}" "$@"

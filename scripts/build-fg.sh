#!/usr/bin/env bash
#
# build-fg.sh — Build the "VelaTerm fg" edition installer, which installs and runs beside the official VelaTerm.
#
# tauri.fg.conf.json gives the edition its own product name, identifier (and so its own data dir, lock and
# credentials), binary name and a disabled updater. VLX_USER_INTEGRATIONS=off leaves the user-global vela command
# and ~/.claude / ~/.codex skills to the primary install, so the two editions do not overwrite each other's copies.
# On a machine where VelaTerm fg is the only edition, build with VLX_USER_INTEGRATIONS=on so it manages them itself.
#
# Output: src-tauri/target/release/bundle/nsis/VelaTerm fg_<version>_x64-setup.exe
#
set -euo pipefail

cd "$(dirname "$0")/.."

[ -d node_modules ] || { echo "==> node_modules is missing, running pnpm install first"; pnpm install; }

case "$(uname -s)" in
  MINGW*|MSYS*|CYGWIN*)
    if [ ! -f src-tauri/resources/gitbash/usr/bin/bash.exe ]; then
      echo "✗ src-tauri/resources/gitbash is empty; copy it from an installed VelaTerm's resources/gitbash first" >&2
      exit 1
    fi ;;
esac

export VLX_USER_INTEGRATIONS="${VLX_USER_INTEGRATIONS:-off}"
exec pnpm tauri build --config src-tauri/tauri.fg.conf.json --bundles nsis "$@"

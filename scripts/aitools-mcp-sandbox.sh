#!/usr/bin/env bash
# Sandboxed launcher for aitools-mcp.
#
# Default scope is the current project only. Extra paths can be added by the
# model via request_access, which OpenCode is configured to ask the user about.
# The bwrap mount namespace limits the server process to ~/Git (read-only)
# plus the project directory (read-write), so editing --root later cannot give
# it access to /etc or the rest of the home directory.
set -euo pipefail

PROJECT="${AITOOLS_PROJECT:-$PWD}"
BIN="${AITOOLS_BIN:-$HOME/.local/bin/aitools-mcp}"

if [[ -n "${AITOOLS_GRANT_BASE:-}" ]]; then
  GRANT_BASE="$AITOOLS_GRANT_BASE"
elif [[ "$PROJECT" == "$HOME/Git/"* ]]; then
  GRANT_BASE="$HOME/Git"
else
  GRANT_BASE="$(dirname "$PROJECT")"
fi

PROJECT="$(realpath -m "$PROJECT")"
GRANT_BASE="$(realpath -m "$GRANT_BASE")"

server_args=(
  --root "$PROJECT"
  --grant-base "$GRANT_BASE"
  --no-inline
  --audit-log "$PROJECT/.aitools/audit.log"
)

bwrap_args=(
  --unshare-all
  --share-net
  --die-with-parent
  --proc /proc
  --dev /dev
  --tmpfs /tmp
  --ro-bind /usr /usr
  --ro-bind /bin /bin
  --ro-bind /lib /lib
  --ro-bind /lib64 /lib64
  --ro-bind /etc /etc
  --ro-bind "$HOME/.local/bin" "$HOME/.local/bin"
  --ro-bind "$GRANT_BASE" "$GRANT_BASE"
)

# The project is writable so request_access can commit its grant file.
if [[ "$PROJECT" != "$GRANT_BASE" ]]; then
  bwrap_args+=(--bind "$PROJECT" "$PROJECT")
fi

exec bwrap "${bwrap_args[@]}" -- "$BIN" "${server_args[@]}"

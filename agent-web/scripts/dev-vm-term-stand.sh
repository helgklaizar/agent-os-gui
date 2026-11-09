#!/usr/bin/env bash
# Stand for the terminal and 429-limit e2e scenarios: a second agentd on the dev VM (port 4097, own DB, own home, non-root runner).
# Usage: DEV_VM_HOST=<ssh host> dev-vm-term-stand.sh start|stop|token. Needs the builds deployed (dev-vm-deploy.sh) and a tunnel `-L 4097:127.0.0.1:4097`.
# `token` prints the stand's web decide token (shown once, do not paste it into chats or reports).
# Optional: DEV_VM_DEST (default agent-web-stand), TERM_STAND_DIR (VM-side work directory, default /tmp/agent-web-term-stand).
set -euo pipefail
HOST="${DEV_VM_HOST:?set DEV_VM_HOST to an ssh host}"
DEST="${DEV_VM_DEST:-agent-web-stand}"
HERE="$(cd "$(dirname "$0")" && pwd)"
W="${TERM_STAND_DIR:-/tmp/agent-web-term-stand}"
case "${1:-start}" in
start | stop)
  ssh "$HOST" "DEST='$DEST' W='$W' CMD='$1' bash -s" <"$HERE/term-stand/remote.sh"
  ;;
token)
  ssh "$HOST" "\$HOME/$DEST/target/release/agentd token issue --name 'e2e-term' --scope chat --ttl-days 1 --decide --socket $W/agentd.sock"
  ;;
*)
  echo "usage: $0 start|stop|token" >&2
  exit 2
  ;;
esac

#!/usr/bin/env bash
# Run Playwright on the dev VM itself: useful where the workstation's browser cannot run (e.g. Firefox on a very new macOS).
# Run from the workstation: copies the scenarios, installs Playwright and a browser on the VM, issues tokens locally (values
# are not printed) and runs the scenarios against 127.0.0.1:4096 and the terminal stand 127.0.0.1:4097 without a tunnel.
# Usage: DEV_VM_HOST=<ssh host> e2e-on-vm.sh [firefox|webkit|chromium] [playwright args...]
# Optional: DEV_VM_DEST (default agent-web-stand), TERM_STAND_DIR (default /tmp/agent-web-term-stand).
set -euo pipefail
HOST="${DEV_VM_HOST:?set DEV_VM_HOST to an ssh host}"
DEST="${DEV_VM_DEST:-agent-web-stand}"
HERE="$(cd "$(dirname "$0")" && pwd)"
W="${TERM_STAND_DIR:-/tmp/agent-web-term-stand}"
BROWSER="${1:-firefox}"
[ "$#" -gt 0 ] && shift
ssh "$HOST" "mkdir -p ~/agent-web-e2e"
rsync -a --exclude node_modules "$HERE/../tests/e2e/" "$HOST:agent-web-e2e/"
bash "$HERE/dev-vm-term-stand.sh" start >/dev/null
ssh "$HOST" "DEST='$DEST' W='$W' BROWSER='$BROWSER' bash -s -- $*" <<'REMOTE'
set -euo pipefail
cd ~/agent-web-e2e
[ -d node_modules ] || npm install --no-audit --no-fund >/dev/null
npx playwright install "$BROWSER" >/dev/null
BIN="$HOME/$DEST/target/release/agentd"
issue() { "$BIN" token issue --name "$1" --scope chat --ttl-days 1 ${2:-} --socket "/run/user/$(id -u)/agentd.sock" | tail -n 1; }
AGENT_WEB_TOKEN="$(issue e2e-vm-chat)"
AGENT_WEB_TOKEN_DECIDE="$(issue e2e-vm-decide --decide)"
AGENT_WEB_TOKEN_TERM="$("$BIN" token issue --name e2e-vm-term --scope chat --ttl-days 1 --decide --socket "$W/agentd.sock" | tail -n 1)"
export AGENT_WEB_TOKEN AGENT_WEB_TOKEN_DECIDE AGENT_WEB_TOKEN_TERM
export AGENT_WEB_URL=http://127.0.0.1:4096 AGENT_WEB_URL_TERM=http://127.0.0.1:4097 AGENT_WEB_PROJECT_TERM="$W/proj"
export AGENT_WEB_BROWSERS="$BROWSER"
npx playwright test "$@"
REMOTE

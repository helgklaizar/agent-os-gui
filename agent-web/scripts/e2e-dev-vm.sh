#!/usr/bin/env bash
# Full run of the browser scenarios against a dev VM: issues tokens on the VM (values stay in this process's variables
# and are never printed), brings up the terminal stand and runs Playwright in all browsers.
# Requires DEV_VM_HOST (an ssh host), a tunnel `ssh -N -L 4096:127.0.0.1:4096 -L 4097:127.0.0.1:4097 $DEV_VM_HOST` and a deployed build (dev-vm-deploy.sh).
# Variables: AGENT_WEB_BROWSERS (default chromium,firefox,webkit), AGENT_WEB_REVOKE=1 (token revocation scenario),
# TERM_STAND_DIR (VM-side stand directory, default /tmp/agent-web-term-stand).
set -euo pipefail
: "${DEV_VM_HOST:?set DEV_VM_HOST to an ssh host}"
HERE="$(cd "$(dirname "$0")" && pwd)"
last() { tail -n 1; }
AGENT_WEB_TOKEN="$(bash "$HERE/dev-vm-token.sh" e2e-chat chat 1 | last)"
AGENT_WEB_TOKEN_DECIDE="$(bash "$HERE/dev-vm-token.sh" e2e-decide chat 1 decide | last)"
export AGENT_WEB_TOKEN AGENT_WEB_TOKEN_DECIDE
if [ "${AGENT_WEB_TERM:-1}" = "1" ]; then
  bash "$HERE/dev-vm-term-stand.sh" start >/dev/null
  AGENT_WEB_TOKEN_TERM="$(bash "$HERE/dev-vm-term-stand.sh" token | last)"
  export AGENT_WEB_TOKEN_TERM
  export AGENT_WEB_URL_TERM="${AGENT_WEB_URL_TERM:-http://127.0.0.1:4097}"
  export AGENT_WEB_PROJECT_TERM="${TERM_STAND_DIR:-/tmp/agent-web-term-stand}/proj"
fi
bash "$HERE/e2e.sh" "$@"

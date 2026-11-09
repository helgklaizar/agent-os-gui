#!/usr/bin/env bash
# Browser scenarios (Playwright) against a running agentd.
# Dependencies are installed into a cache outside the repository tree (`~/.cache/agent-web-e2e`), not part of cargo or the build.
# Variables: AGENT_WEB_TOKEN (required, a `chat` token), AGENT_WEB_TOKEN_DECIDE (web token with the decide right, optional),
# AGENT_WEB_REVOKE=1 enables the token revocation scenario (tokens are issued and revoked over ssh on the dev VM via
# scripts/dev-vm-token.sh and `agentd token`, so DEV_VM_HOST must be set).
# AGENT_WEB_URL (default http://127.0.0.1:4096; needs a tunnel to the VM when remote). Token values are never printed.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
export AGENT_WEB_SCRIPTS="$HERE"
cd "$HERE/../tests/e2e"
CACHE="${AGENT_WEB_E2E_DIR:-$HOME/.cache/agent-web-e2e}"
mkdir -p "$CACHE"
rsync -a --exclude node_modules ./ "$CACHE/"
cd "$CACHE"
[ -d node_modules ] || npm install --no-audit --no-fund >/dev/null
npx playwright install chromium firefox webkit >/dev/null
exec npx playwright test "$@"

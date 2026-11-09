#!/usr/bin/env bash
# Demo model setup on the dev VM: a `scripted` account, the `scripted` model on the models page, and a "Demo" persona using it.
# Works only with a test-support build (DEMO=1 in dev-vm-deploy.sh). The admin token comes from agentd via gui-launch
# (a file descriptor, the value never reaches the chat); serve runs with --launch-ancestor sshd for that (stand only).
# Requires: DEV_VM_HOST (an ssh host); optional DEV_VM_DEST (default agent-web-stand).
set -euo pipefail
HOST="${DEV_VM_HOST:?set DEV_VM_HOST to an ssh host}"
DEST="${DEV_VM_DEST:-agent-web-stand}"
ssh "$HOST" bash -s <<REMOTE
set -euo pipefail
S="/run/user/\$(id -u)/agentd.sock"
\$HOME/$DEST/target/release/agentd gui-launch --launch-socket "/run/user/\$(id -u)/agentd-launch.sock" -- bash -c '
set -euo pipefail
T=\$(cat /proc/self/fd/\$AGENTD_TOKEN_FD)
api() { curl -s --unix-socket "'"\$S"'" -H "Authorization: Bearer \$T" -H "Content-Type: application/json" "\$@"; }
acc=\$(api -X POST http://localhost/v1/accounts -d "{\"type\":\"scripted\",\"label\":\"demo\"}" | sed -E "s/.*\"id\":([0-9]+).*/\1/")
rev=\$(api http://localhost/v1/settings/models | sed -E "s/.*\"rev\":([0-9]+).*/\1/")
api -X PATCH http://localhost/v1/settings/models -H "If-Match: \$rev" -d "{\"accounts\":{\"\$acc\":{\"model\":\"scripted\"}}}" >/dev/null
api -X POST http://localhost/v1/personas -d "{\"name\":\"Demo\",\"color\":\"#4c8dff\",\"model_acc\":\$acc,\"lang\":\"en\"}" | head -c 120; echo
'
REMOTE

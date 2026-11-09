#!/usr/bin/env bash
# Issue a browser-client token on the dev VM (run on the VM over ssh; the value is printed once, do not paste it into chats or reports).
# Usage: DEV_VM_HOST=<ssh host> dev-vm-token.sh [name] [chat|read] [days] [decide]
#   with `decide` the web token may decide permission requests one at a time: `agentd token issue --decide`, up to 30 days.
# Optional: DEV_VM_DEST (deploy directory relative to the VM home, default agent-web-stand).
set -euo pipefail
HOST="${DEV_VM_HOST:?set DEV_VM_HOST to an ssh host}"
DEST="${DEV_VM_DEST:-agent-web-stand}"
NAME="${1:-web}"; SCOPE="${2:-chat}"; DAYS="${3:-30}"; MODE="${4:-}"
FLAG=""; [ "$MODE" = "decide" ] && FLAG="--decide"
ssh "$HOST" "\$HOME/$DEST/target/release/agentd token issue --name '$NAME' --scope '$SCOPE' --ttl-days '$DAYS' $FLAG --socket \"/run/user/\$(id -u)/agentd.sock\""

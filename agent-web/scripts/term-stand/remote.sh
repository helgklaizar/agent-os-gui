#!/usr/bin/env bash
# VM side of the terminal stand: a separate agentd on 127.0.0.1:4097 with its own DB and home, a non-root runner.
# Called by agent-web/scripts/dev-vm-term-stand.sh; variables: DEST (deploy directory), W (stand work directory), CMD (start|stop).
set -euo pipefail
DEST="${DEST:-agent-web-stand}"
W="${W:-/tmp/agent-web-term-stand}"
BIN="$HOME/$DEST/target/release/agentd"
UID_N="$(id -u)"
case "${CMD:-start}" in
stop)
  systemctl --user stop agent-web-term-agentd agent-web-term-runner 2>/dev/null || true
  systemctl --user reset-failed agent-web-term-agentd agent-web-term-runner 2>/dev/null || true
  exit 0
  ;;
esac
systemctl --user stop agent-web-term-agentd agent-web-term-runner 2>/dev/null || true
systemctl --user reset-failed agent-web-term-agentd agent-web-term-runner 2>/dev/null || true
mkdir -p "$W/home/.agentos" "$W/proj"
rm -f "$W/sessions.db" "$W/sessions.db-wal" "$W/sessions.db-shm"
cp "$HOME/$DEST/agent-web/scripts/term-stand/runner.py" "$W/runner.py"
systemd-run --user --quiet --unit=agent-web-term-runner python3 "$W/runner.py" "$W/runner.sock" "$BIN" "$UID_N" "$W/proj"
sleep 1
systemd-run --user --quiet --unit=agent-web-term-agentd \
  --setenv=HOME="$W/home" --setenv=AGENTD_RUNNER_SOCKET="$W/runner.sock" \
  "$BIN" serve --db "$W/sessions.db" --socket "$W/agentd.sock" --tcp 127.0.0.1:4097 \
  --web-root "$HOME/$DEST/agent-web/dist" --launch-socket "$W/launch.sock" --launch-ancestor sshd
# Startup with migrations takes a few seconds: wait for /health (up to 30 s; sockets of the previous run stay on disk).
for _ in $(seq 60); do curl -sf --unix-socket "$W/agentd.sock" http://localhost/v1/health >/dev/null 2>&1 && break; sleep 0.5; done
sleep 3
systemctl --user is-active agent-web-term-agentd agent-web-term-runner >/dev/null
# Account and "Demo" persona (deterministic scripted provider, as on the main stand).
setup() { "$BIN" gui-launch --launch-socket "$W/launch.sock" -- bash -c '
set -euo pipefail
T=$(cat /proc/self/fd/$AGENTD_TOKEN_FD)
api() { curl -s --unix-socket "'"$W"'/agentd.sock" -H "Authorization: Bearer $T" -H "Content-Type: application/json" "$@"; }
acc=$(api -X POST http://localhost/v1/accounts -d "{\"type\":\"scripted\",\"label\":\"demo\"}" | sed -E "s/.*\"id\":([0-9]+).*/\1/")
rev=$(api http://localhost/v1/settings/models | sed -E "s/.*\"rev\":([0-9]+).*/\1/")
api -X PATCH http://localhost/v1/settings/models -H "If-Match: $rev" -d "{\"accounts\":{\"$acc\":{\"model\":\"scripted\"}}}" >/dev/null
api -X POST http://localhost/v1/personas -d "{\"name\":\"Demo\",\"color\":\"#4c8dff\",\"model_acc\":$acc,\"lang\":\"en\"}" >/dev/null
'; }
# The GUI launch listener comes up slightly after the API: retry up to 30 times.
for _ in $(seq 30); do setup 2>/dev/null && break; sleep 1; done
echo "terminal stand: http://127.0.0.1:4097, project $W/proj"

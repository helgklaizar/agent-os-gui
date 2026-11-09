#!/bin/bash
# Install agent-watch (A7-03): sudo bash agentd/packaging/agent-watch-install.sh <owner-login> [rules-dir]
# Needs root. Deletes nothing; previous configuration files are moved to *.bak-YYYYMMDD.
# After installing, set in the core unit (~/.config/systemd/user/agentd.service.d/watch.conf)
#   [Service]
#   Environment=AGENTD_WATCH_SOCKET=/run/agent-watch.sock
set -euo pipefail
[ "$(id -u)" = 0 ] || { echo "root required: sudo bash $0 <owner> [rules-dir]" >&2; exit 1; }
OWNER=${1:?owner login required}
RULES=${2:-}
UID_N=$(id -u "$OWNER")
DATE=$(date +%Y%m%d)
SRC=$(cd "$(dirname "$0")/deb/usr/lib" && pwd)
backup() { if [ -e "$1" ]; then mv "$1" "$1.bak-$DATE"; fi; }
systemd-sysusers "$SRC/sysusers.d/agentd.conf"
install -m 0644 "$SRC/systemd/system/agent-watch.socket" /etc/systemd/system/agent-watch.socket
install -m 0644 "$SRC/systemd/system/agent-watch@.service" /etc/systemd/system/agent-watch@.service
ARGS="--allow-uid $UID_N"
if [ -n "$RULES" ]; then
    ARGS="$ARGS --rules-dir $(realpath "$RULES")"
fi
backup /etc/agent-watch.env
printf 'WATCH_ARGS=%s\n' "$ARGS" > /etc/agent-watch.env
chmod 0644 /etc/agent-watch.env
mkdir -p /etc/systemd/system/agent-watch.socket.d
printf '[Socket]\nSocketUser=%s\n' "$OWNER" > /etc/systemd/system/agent-watch.socket.d/owner.conf
systemctl daemon-reload
systemctl enable --now agent-watch.socket
echo "done: socket /run/agent-watch.sock; for the core set AGENTD_WATCH_SOCKET=/run/agent-watch.sock"

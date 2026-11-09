#!/bin/bash
# Install agent-runner (A7-01): sudo bash agentd/packaging/agent-runner-install.sh <owner-login> <project-dir>...
# Needs root. Deletes nothing; previous configuration files are moved to *.bak-YYYYMMDD.
set -euo pipefail
[ "$(id -u)" = 0 ] || { echo "root required: sudo bash $0 <owner> <dir>..." >&2; exit 1; }
OWNER=${1:?owner login required}
shift
[ "$#" -ge 1 ] || { echo "at least one project directory is required" >&2; exit 1; }
UID_N=$(id -u "$OWNER")
DATE=$(date +%Y%m%d)
SRC=$(cd "$(dirname "$0")/deb/usr/lib" && pwd)
backup() { if [ -e "$1" ]; then mv "$1" "$1.bak-$DATE"; fi; }
systemd-sysusers "$SRC/sysusers.d/agentd.conf"
install -m 0644 "$SRC/systemd/system/agent-runner.socket" /etc/systemd/system/agent-runner.socket
install -m 0644 "$SRC/systemd/system/agent-runner@.service" /etc/systemd/system/agent-runner@.service
ARGS="--allow-uid $UID_N"
RW=""
for d in "$@"; do
    REAL=$(realpath "$d")
    ARGS="$ARGS --allow-dir $REAL"
    RW="$RW $REAL"
done
backup /etc/agent-runner.env
printf 'RUNNER_ARGS=%s\n' "$ARGS" > /etc/agent-runner.env
chmod 0644 /etc/agent-runner.env
mkdir -p /etc/systemd/system/agent-runner.socket.d /etc/systemd/system/agent-runner@.service.d
printf '[Socket]\nSocketUser=%s\n' "$OWNER" > /etc/systemd/system/agent-runner.socket.d/owner.conf
printf '[Service]\nReadWritePaths=%s\n' "${RW# }" > /etc/systemd/system/agent-runner@.service.d/rw.conf
systemctl daemon-reload
systemctl enable --now agent-runner.socket
echo "done: verify with bash $(dirname "$0")/agent-runner-verify.sh <project-dir>"

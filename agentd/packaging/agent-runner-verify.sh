#!/bin/bash
# Check agent-runner (A7-01): bash agentd/packaging/agent-runner-verify.sh <project-dir>
# Run as the owner user. Exit 0: all checks passed.
set -uo pipefail
DIR=${1:?an allowed project directory is required}
SOCK=/run/agent-runner.sock
FAIL=0
ok() { echo "ok   $1"; }
bad() { echo "FAIL $1"; FAIL=1; }
ask() {
    python3 -I -c '
import socket, sys
s = socket.socket(socket.AF_UNIX)
s.settimeout(15)
s.connect(sys.argv[1])
s.sendall((sys.argv[2] + "\n").encode())
buf = b""
while True:
    d = s.recv(65536)
    if not d:
        break
    buf += d
sys.stdout.write(buf.decode(errors="replace"))
' "$SOCK" "$1"
}
if systemctl is-active --quiet agent-runner.socket; then ok "socket is active"; else bad "socket is not active"; fi
if [ "$(stat -c %a "$SOCK" 2>/dev/null)" = 600 ]; then ok "socket mode 0600"; else bad "socket mode"; fi
R=$(ask "{\"cwd\":\"$DIR\",\"command\":\"echo hello\"}")
if echo "$R" | grep -q '"data":"hello' && echo "$R" | grep -q '"exit":0'; then ok "command in project"; else bad "command in project: $R"; fi
R=$(ask '{"cwd":"/etc","command":"true"}')
if echo "$R" | grep -q '"denied":"dir"'; then ok "/etc rejected"; else bad "/etc rejected: $R"; fi
R=$(ask "{\"cwd\":\"$DIR\",\"command\":\"true\",\"uid\":0}")
if echo "$R" | grep -q '"denied":"bad_request"'; then ok "extra field rejected"; else bad "extra field: $R"; fi
R=$(ask "{\"cwd\":\"$DIR\",\"command\":\"sleep 30\",\"timeout_secs\":1}")
if echo "$R" | grep -q '"timeout":true'; then ok "timeout"; else bad "timeout: $R"; fi
exit $FAIL

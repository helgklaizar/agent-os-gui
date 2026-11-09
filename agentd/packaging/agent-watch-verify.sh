#!/bin/bash
# Check agent-watch isolation (A7-03, watch_no_secrets_access): sudo bash agentd/packaging/agent-watch-verify.sh <owner-login>
# Needs root and installed units. Runs commands UNDER THE PARAMETERS of the agent-watch@.service unit (a transient unit with the same
# restrictions) and checks that keys and network are unreachable. Changes and deletes nothing. Exit 0: everything is unreachable, as required.
set -uo pipefail
OWNER=${1:?owner login required}
HOME_DIR=$(getent passwd "$OWNER" | cut -d: -f6)
FAIL=0
ok() { echo "ok   $1"; }
bad() { echo "FAIL $1"; FAIL=1; }
RUN=(systemd-run --wait --pipe --quiet -p User=agentos-watch -p NoNewPrivileges=true -p PrivateNetwork=true
    -p ProtectHome=true -p InaccessiblePaths=/run/user -p ProtectSystem=strict -p RestrictAddressFamilies=AF_UNIX
    -p CapabilityBoundingSet=)
if "${RUN[@]}" /bin/sh -c "cat '$HOME_DIR/.agentos/keys.env'" >/dev/null 2>&1; then bad "keys.env is readable"; else ok "keys.env is unreachable"; fi
if "${RUN[@]}" /bin/sh -c "ls /run/user" >/dev/null 2>&1; then bad "/run/user is visible"; else ok "/run/user is unreachable"; fi
if "${RUN[@]}" /bin/sh -c "exec 3<>/dev/tcp/1.1.1.1/53" >/dev/null 2>&1; then bad "network is reachable"; else ok "network is unreachable"; fi
if "${RUN[@]}" /bin/sh -c "touch /etc/agent-watch-probe" >/dev/null 2>&1; then bad "write to /etc"; else ok "write to /etc is blocked"; fi
exit $FAIL

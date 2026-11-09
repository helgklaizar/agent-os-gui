#!/usr/bin/env bash
# Deploy agentd and the browser client to a dev VM. Run from the repository root on the workstation.
#  1) builds the client (trunk), 2) rsyncs the sources to ~/$DEV_VM_DEST on the VM (no deletion), 3) builds agentd on the VM,
#  4) installs the user unit agentd-web and restarts it.
# Requires: DEV_VM_HOST (an ssh host); optional DEV_VM_DEST (default agent-web-stand).
# Built with `test-support`: the stand needs the deterministic `scripted` provider (chat demo without a key). For a
# production-like build disable the feature: DEMO=0.
# The script contains and prints no secrets; issue a token separately (agent-web/scripts/dev-vm-token.sh).
set -euo pipefail
cd "$(dirname "$0")/../.."
HOST="${DEV_VM_HOST:?set DEV_VM_HOST to an ssh host}"
DEST="${DEV_VM_DEST:-agent-web-stand}"
FEATURES=""
[ "${DEMO:-1}" = "1" ] && FEATURES="--features test-support"
bash agent-web/scripts/build.sh --release
ssh "$HOST" "mkdir -p ~/$DEST ~/.agentos"
rsync -a --exclude .git --exclude target --exclude .claude --exclude agent-web/target \
  ./ "$HOST:$DEST/"
ssh "$HOST" bash -s <<REMOTE
set -euo pipefail
export PATH="\$HOME/.cargo/bin:\$PATH"
cd ~/$DEST
cargo build --release -p agentd $FEATURES 2>&1 | tail -3
mkdir -p ~/.config/systemd/user
cat > ~/.config/systemd/user/agentd-web.service <<UNIT
[Unit]
Description=agentd with the browser client (dev VM)

[Service]
ExecStart=%h/$DEST/target/release/agentd serve --db %h/.agentos/sessions.db --socket %t/agentd.sock --tcp 127.0.0.1:4096 --web-root %h/$DEST/agent-web/dist --launch-ancestor sshd
Restart=on-failure
NoNewPrivileges=yes

[Install]
WantedBy=default.target
UNIT
systemctl --user daemon-reload
systemctl --user enable agentd-web.service >/dev/null 2>&1 || true
systemctl --user restart agentd-web.service
sleep 2
systemctl --user is-active agentd-web.service
curl -s -o /dev/null -w "page: HTTP %{http_code}\n" http://127.0.0.1:4096/
REMOTE

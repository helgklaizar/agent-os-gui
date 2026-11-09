#!/usr/bin/env bash
# Package the browser client: the built `dist` plus a VERSION file (client version and the API major it was built for).
# Output: agent-web/pkg/agent-web-<version>.tar.gz. The directory is installed as the core's `--web-root`, e.g.
# /usr/share/agentd/web; client and core are released together (one workspace version).
set -euo pipefail
cd "$(dirname "$0")/.."
bash scripts/build.sh --release >/dev/null
VER="$(cargo pkgid -p agent-web | sed -E 's/.*[#@]//')"
rm -rf pkg/stage && mkdir -p pkg/stage
cp -R dist/. pkg/stage/
printf 'agent-web %s\napi-major 1\n' "$VER" > pkg/stage/VERSION
tar -czf "pkg/agent-web-$VER.tar.gz" -C pkg/stage .
echo "pkg/agent-web-$VER.tar.gz"

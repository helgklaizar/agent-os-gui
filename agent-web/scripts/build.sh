#!/usr/bin/env bash
# Build the browser client: trunk, then move the inline bootstrap script into boot.js so the page CSP
# needs no 'unsafe-inline' (script-src 'self' 'wasm-unsafe-eval').
# Usage: scripts/build.sh [--release|--debug]   Output: agent-web/dist/
set -euo pipefail
cd "$(dirname "$0")/.."
mode="${1:---release}"
# Leftovers of earlier builds (hashed file names) are moved aside instead of piling up in dist.
if [ -d dist ]; then trash dist 2>/dev/null || mv dist "dist.bak-$(date +%Y%m%d%H%M%S)"; fi
if [ "$mode" = "--debug" ]; then trunk build; else trunk build --release --cargo-profile web; fi
python3 -I - <<'PY'
import re, pathlib
p = pathlib.Path("dist/index.html")
s = p.read_text(encoding="utf-8")
m = re.search(r'<script type="module">(.*?)</script>', s, re.S)
if not m:
    raise SystemExit("inline bootstrap script not found: this trunk version changed the markup")
pathlib.Path("dist/boot.js").write_text(m.group(1).strip() + "\n", encoding="utf-8")
s = s.replace(m.group(0), '<script type="module" src="/app/boot.js"></script>')
if "<script>" in s or re.search(r"<script(?![^>]*\bsrc=)", s):
    raise SystemExit("an inline script is left in index.html")
p.write_text(s, encoding="utf-8")
print("dist/boot.js extracted, no inline scripts left")
PY

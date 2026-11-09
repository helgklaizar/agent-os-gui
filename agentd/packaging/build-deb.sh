#!/bin/bash
# Build the core .deb: bash agentd/packaging/build-deb.sh <output-dir>  (run on dev-vm from the repository root).
# Version: <semver from Cargo.toml>-<revision>~ubuntu<release>; the binary is built with `cargo build --release --locked -p agentd`.
set -euo pipefail
OUT=${1:?output directory required}
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
SEMVER=$(sed -n 's/^version = "\(.*\)"/\1/p' "$ROOT/Cargo.toml" | head -1)
SEMVER=${SEMVER:-0.0.0}
REV=${REVISION:-1}
REL=$(lsb_release -rs 2>/dev/null || echo 0)
ARCH=$(dpkg --print-architecture)
VERSION="${SEMVER/-/~}-${REV}~ubuntu${REL}"   # semver pre-release "-alpha" becomes Debian "~alpha" (sorts before the release)
cargo build --release --locked -p agentd --manifest-path "$ROOT/Cargo.toml"
BIN=${CARGO_TARGET_DIR:-$ROOT/target}/release/agentd
STAGE=$(mktemp -d)
cp -a "$ROOT/agentd/packaging/deb/." "$STAGE/"
# Browser client: built `agent-web/dist` -> /usr/share/agentd/web; without it the core package is built as before.
WEB="$ROOT/agent-web/dist"
if [ "${WITH_WEB:-1}" = "1" ] && [ ! -f "$WEB/index.html" ] && command -v trunk >/dev/null 2>&1; then
  bash "$ROOT/agent-web/scripts/build.sh" --release
fi
if [ "${WITH_WEB:-1}" = "1" ] && [ -f "$WEB/index.html" ]; then
  mkdir -p "$STAGE/usr/share/agentd/web"
  # Only files referenced by index.html and boot.js (dist may contain leftovers of earlier builds).
  cp "$WEB/index.html" "$STAGE/usr/share/agentd/web/"
  for f in boot.js $(cat "$WEB/index.html" "$WEB/boot.js" | grep -o "/app/[A-Za-z0-9_.-]*" | sed "s|/app/||" | sort -u); do
    [ -f "$WEB/$f" ] && cp "$WEB/$f" "$STAGE/usr/share/agentd/web/"
  done
  printf 'agent-web %s\napi-major 1\n' "$SEMVER" > "$STAGE/usr/share/agentd/web/VERSION"
fi
# Package documentation (lintian): copyright lives in the package tree, the changelog is generated for the version.
mkdir -p "$STAGE/usr/share/doc/agentd"
chmod 0644 "$STAGE/usr/share/doc/agentd/copyright"
printf 'agentd (%s) unstable; urgency=low\n\n  * Agent OS core build.\n\n -- Agent OS <agentos@agentos.example.org>  %s\n' \
  "$VERSION" "$(date -R)" | gzip -9n > "$STAGE/usr/share/doc/agentd/changelog.Debian.gz"
mkdir -p "$STAGE/usr/bin"
install -m 0755 "$BIN" "$STAGE/usr/bin/agentd"
strip "$STAGE/usr/bin/agentd"
sed -e "s/@VERSION@/$VERSION/" -e "s/@ARCH@/$ARCH/" "$STAGE/DEBIAN/control.in" > "$STAGE/DEBIAN/control"
mv "$STAGE/DEBIAN/control.in" "$STAGE.control.in"
chmod -R go-w "$STAGE"
chmod 0755 "$STAGE/DEBIAN/postinst" "$STAGE/DEBIAN/prerm" "$STAGE/DEBIAN/postrm"
mkdir -p "$OUT"
dpkg-deb --root-owner-group --build "$STAGE" "$OUT/agentd_${VERSION}_${ARCH}.deb"

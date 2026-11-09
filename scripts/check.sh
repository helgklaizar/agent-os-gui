#!/usr/bin/env bash
# Project check entry point. CHECK_STRICT=1 makes missing optional tools (gitleaks, cargo-deny) a failure.
set -uo pipefail
cd "$(dirname "$0")/.." || exit 1
fail=0
step() { echo "== $1"; shift; "$@" || { echo "FAILED: $*" >&2; fail=1; }; }
skip_or_fail() { if [ "${CHECK_STRICT:-0}" = 1 ]; then echo "FAILED: $1 is not installed (CHECK_STRICT=1)" >&2; fail=1; else echo "skipped: $1 is not installed"; fi; }

step "hook syntax" bash -n .githooks/pre-commit
step "script syntax" bash -n scripts/check.sh

# shellcheck source=secret-pattern.sh
. scripts/secret-pattern.sh
echo "== secret patterns over the tree"
if grep -rIEl --exclude-dir=target --exclude-dir=.git --exclude-dir=node_modules --exclude=secret-pattern.sh --exclude=pre-commit --exclude=check.sh -e "$SECRET_PATTERN" . >/dev/null 2>&1; then
	echo "FAILED: secret-like strings found" >&2; fail=1
fi

step "cargo fmt" cargo fmt --all -- --check
step "cargo clippy" cargo clippy --locked --workspace --all-targets -- -D warnings
step "cargo test" cargo test --locked --workspace
if command -v cargo-deny >/dev/null 2>&1; then step "cargo deny" cargo deny --locked check; else skip_or_fail cargo-deny; fi
if command -v gitleaks >/dev/null 2>&1; then step "gitleaks" gitleaks dir . --no-banner --redact; else skip_or_fail gitleaks; fi

if [ "$fail" = 0 ]; then echo "ALL OK"; else echo "CHECK FAILED" >&2; exit 1; fi

# Testing

Status: pre-alpha (0.0.1-alpha). This document covers the test strategy for the GUI (`agent-shell`, `agent-web`, shared client crates), the mock core `fake-agentd` and the core `agentd`.

## 1. Strategy

The client is thin: all state logic lives in controllers, widgets only render a model. Most tests therefore target controllers without GTK. A bug is not fixed until a regression test for it exists.

| Level | What it checks | Environment |
|---|---|---|
| 1. Unit | controllers, view models, pure functions (colour thresholds, escaping, persona inheritance, permissions, address parsing, cron and time zones, token hashing) | `cargo test`, no GTK, no network |
| 2. Integration | controller + core client against `fake-agentd` (HTTP + WebSocket); the core as a library against a real SQLite file (migrations, triggers, the write queue, retention, `VACUUM INTO`) | `cargo test`, local socket |
| 3. Contract | every route and WebSocket frame against `docs/reference/openapi-draft.yaml`, with the same suite run against both `fake-agentd` and the real core | local socket and TCP loopback |
| 4. System (Linux with systemd) | `agentd runner`, the unprivileged agent user, descriptor-based GUI token issue, `SO_PEERCRED`, units, watchdog | a Linux VM (`dev-vm`); not macOS, not a container without systemd |
| 5. UI scenarios | four-column layout, dialogs, Settings, keyboard, themes, accessibility tree | GTK under headless Wayland (`weston --backend=headless`), our own runner because GTK lives in one thread |
| 6. Browser end-to-end | the `agent-web` scenarios in chromium, firefox and webkit | Playwright against a live `agentd` |

Principles:

- **A test must go red when the product breaks.** Critical tests are checked by breaking a line of product code (red) and restoring it (green); long series use mutation testing.
- **No record, no success.** Mutation tests assert on database state, not only on the response code.
- **The core does not know it is tested.** There are no test branches in product code. The one accepted exception is the `test` token kind and controllable clocks, which exist only in a build with the `test-support` feature.
- **Real SQLite, not a database mock.** Only the HTTP boundary of providers (models, messengers, MCP) is replaced, and only inside `tests/`.
- **Time and randomness are injected** (clocks, token generation, reconnect jitter).
- **Test data is synthetic.** No personal data or real secrets in fixtures; fixture secrets share a prefix so that a grep can detect a leak in logs.
- Reports contain only numbers that come from a log taken after the last code commit.

## 2. Commands

```bash
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace                      # unit and integration, including fake-agentd
cargo test --locked -p integration-tests             # controllers and client against fake-agentd
cargo test --locked -p agentd-e2e                    # real agentd process, contract tests
cargo test --locked -p controllers -p core-client -p fake-agentd   # without GTK
cargo deny check                                     # licenses, sources, advisories (deny.toml)
cargo mutants -p controllers -p core-client          # mutation testing, nightly or --in-diff
bash scripts/check.sh                                # single entry point for the project checks
CHECK_STRICT=1 bash scripts/check.sh                 # CI mode: a missing optional tool is a failure
```

Web client:

```bash
cargo test --locked -p agent-web
cargo clippy --locked -p agent-web --target wasm32-unknown-unknown --all-targets -- -D warnings
agent-web/scripts/e2e.sh                             # Playwright; see agent-web/README.md for variables
```

UI scenarios (Linux only):

```bash
weston --backend=headless --socket=agent-test --width=1024 --height=640 &
WAYLAND_DISPLAY=agent-test GDK_BACKEND=wayland cargo test --locked -p ui --test scenarios
```

On macOS only the GTK-free crates are tested: `controllers`, `core-client`, `fake-agentd`, `agent-web` (host tests). The `ui` crate and the UI scenarios need Linux.

Lint rules: `unsafe_code = "forbid"` for the whole workspace (one documented exception in `agent-shell/ui/src/hardening.rs`); `#![deny(clippy::unwrap_used)]` in `core-client` and `controllers` (tests may unwrap through `clippy.toml`); `clippy.toml` also forbids HTML insertion APIs in the browser client.

## 3. The mock core `fake-agentd`

A separate crate, usable as a library inside a test and as a binary for manual work. It does not own the schema: contract tests keep it in line with the OpenAPI draft and with the real core.

- Transport: unix socket and optionally TCP on `127.0.0.1` with a random port. Clients find it through `AGENTD_SOCKET` (path, takes precedence) or `AGENTD_ADDR` (`tcp:127.0.0.1:port`).
- API: the HTTP routes and WebSocket topics of [api.md](reference/api.md) (`bar.*`, `events`, `session.<id>`, `sessions`, `modules`, `settings`, `connections`, ...).
- Scenarios: declarative TOML files in `fake-agentd/scenarios/` describing frame sequences with delays; the same scenario yields the same result. Time is controlled with `--clock manual` and advanced through `POST /_test/advance`.
- Tokens: the `gui` token can be written to a file (`--token-file`, `AGENTD_TOKEN_FILE` on the client side) in test builds only.
- Hostile data: test helpers provide "evil" strings (markup, control characters, very long text) used by escaping tests.

## 4. What the GUI tests cover

By area (each controller has unit tests; integration tests live in `agent-shell/integration-tests/tests/`):

- **Shell and layout.** The layout is a pure function of window width: auto-collapse thresholds, minimum chat width, `ui.json` round trip, corrupted file falls back to defaults, atomic write.
- **Bar.** Colour thresholds (CPU/RAM/disk 50/80, temperature 60/80, boundary values with half-up rounding), quota forecast only when remaining quota and reset time are known, indicator collapse by priority, grey when data is missing, labels from the core are escaped.
- **Notifications.** Three-level filters, counters, mark-as-read, window title, "open chat" navigation to the right session and tab.
- **Session tree.** Create/move/rename/delete, selection by identifier after deletion, persona inheritance, unread badges.
- **Chat.** Idempotent send by `client_id`, streaming with `offset`, resync from a snapshot, stop, Markdown rendering without HTML, permission and observer cards, first-decision-wins (`409`).
- **Settings.** Real saving, `If-Match` and `412` handling, secrets are write-only and never echoed, error display.
- **Connection.** All link states, reconnect without attempt limit, the outgoing queue (TTL, revalidation, manual cancel), no duplicate after retry.
- **Theme and shortcuts.** Core is the source of truth, cache in `ui.json`; shortcuts do not fire inside terminal/browser where reserved.
- **Escaping.** Every place that shows external text is exercised with hostile strings in controller tests and in UI scenarios (no markup interpretation, no scheme other than `http`, `https`, `mailto` opened).
- **Accessibility.** Accessible names for icon buttons, keyboard reachability, contrast of theme tokens (WCAG AA) checked on the token tables, AT-SPI tree roles and states in UI scenarios.
- **Hardening.** The GUI process is not dumpable and avoids portals (`hardening.rs`); the token is read once from the descriptor and `Debug` output never prints it.

## 5. What the core tests cover

| Subsystem | What is tested | How |
|---|---|---|
| API (HTTP/WS) | routes, error codes, `X-Request-Id`, `If-Match` (412/428), error envelope, WebSocket topics, `seq`/`offset`, close codes (`1001`, `4401`, `4403`), `503 migrating` | contract tests against the OpenAPI draft + integration |
| Tokens and scope | SHA-256 at rest, shown once, `?token=` gives `401`, expiry, revocation, `external` never `admin`, allowlists, `429` with `Retry-After`, constant-time compare | unit + integration with SQLite |
| GUI token issue | the token is absent from environment, command line and file descriptors listing; a second issue is rejected; a request not from the expected ancestor is rejected | system level |
| Permissions | strictest of three levels, `403` plus observer finding on an attempt to weaken, "always", first-wins (`409`), 10 minute TTL, fail-closed with an unavailable observer | unit tables + integration |
| Database and single writer | one write connection, read-only pool, WAL, file modes, triggers, `streaming` to `interrupted` at start | integration + system |
| Migrations | up from zero, idempotent re-run, database newer than code is refused, pre-migration snapshot, `foreign_key_check`, free disk check | integration |
| Providers | response parsing, `usage`, rate-limit headers, 4xx/5xx, timeouts, retry, fallback | HTTP mocks |
| Streaming | delta order, `offset`, `final_len`, mid-stream drop, resubscribe with snapshot, `interrupted` | scripted provider + WebSocket client |
| Scheduler | cron in the task time zone, UTC `next_run`, missed runs, paused tasks, duplicate triggers | controllable clock |
| Untrusted input | `untrusted` marking of mail, pages, tool results; the source is shown in a permission request; no privilege escalation from external text | integration (the detection quality of a real local model is not measured) |
| Agent runner | directory allowlist, `SO_PEERCRED`, `..` and symlinks, extra parameters, fail-closed when the runner is stopped | system level |
| Secrets | write-only, never in logs, database, diagnose report or WebSocket | integration + grep over logs |
| Tunnels and TCP | listens only when enabled; `Origin`/`Host` allowlist; `X-Forwarded-For` right to left; `logout`, `files`, `secrets`, `permissions` and token issue unavailable over TCP | TCP loopback integration |
| Backup | `VACUUM INTO` without blocking the writer, `age` encryption with two recipients, restore moves live files aside instead of deleting | system / semi-automatic |
| Single writer | under parallel mutations there is one write connection and no `SQLITE_BUSY` on write; the read pool refuses writes; no other process opens the database for writing | integration + system |

Mutation testing targets: token/scope/expiry checks, effective permission, idempotency (`client_id`), stream `offset`, write queue and migration runner, path validation, allowlists, secret redaction. Load scenarios (mixed clients, stream fan-out, database scale, reconnect storm, authentication flood, 24 hour soak) measure latency percentiles, memory and WAL growth; numeric thresholds are set from measurement, not assumed.

## 6. The check script

`scripts/check.sh` is the single entry point. Each step prints what it verified; any failure gives a non-zero exit code. It runs, in order: documentation link check, a scan for secrets over the tree, the cargo steps (`fmt --check`, `clippy -D warnings`, `test --locked --workspace`, `cargo deny check`; a missing `cargo-deny` is a failure, not a skip), the wasm32 check of `agent-web`, and a secret scanner such as `gitleaks` when installed (mandatory with `CHECK_STRICT=1`). The script also prints what it does not cover: UI scenarios under headless Wayland, mutation testing, acceptance runs against a real core, and secrets in git history.

## 7. Pass criteria

- Levels 1 to 3 green; `cargo fmt` and `clippy -D warnings` clean; `cargo deny check` green.
- Every known bug has a regression test.
- Mutation score on security-critical code is reviewed per release; surviving mutants are triaged, not ignored.

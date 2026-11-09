# Code map

Status: pre-alpha (0.0.1-alpha). Verified against the source tree. All crates are members of one Cargo workspace ([Cargo.toml](../../Cargo.toml)). Layers and dependency rules: [architecture](../architecture.md). Tests: [testing](../testing.md).

## 1. Crates

| Crate | Path | What | GTK |
|---|---|---|---|
| `core-client` | `agent-shell/core-client` | client of the core: HTTP and WebSocket over unix socket and TCP, token handling, outgoing queue | no |
| `controllers` | `agent-shell/controllers` | pure client logic: controllers, event bus, layout, thresholds, i18n, view models | no |
| `ui` (binary `agent-shell`) | `agent-shell/ui` | GTK4 + libadwaita window, widgets, bridge to controllers | Linux only |
| `integration-tests` | `agent-shell/integration-tests` | controllers and client against `fake-agentd` and against a real `agentd` | no |
| `agent-web` | `agent-web` | browser client (Leptos, wasm32), built with `trunk` | no |
| `fake-agentd` | `fake-agentd` | scripted mock of the core (library and binary) | no |
| `agentd-core` | `agentd/core` | domain types, ports, orchestrator, permission engine, observer, gateway, MCP client, indicators, scheduler logic | no |
| `agentd-db` | `agentd/db` | SQLite: single writer, readers, migrations, repositories, backup, integrity, retention | no |
| `agentd-api` | `agentd/api` | HTTP and WebSocket server: routes, authentication layers, hub, topics | no |
| `agentd-auth` | `agentd/auth` | token issue, hashing and verification | no |
| `agentd-providers` | `agentd/providers` | Anthropic and OpenAI-compatible providers, SSE, MCP transports, messenger channels, weather | no |
| `agentd-sys` | `agentd/sys` | safe wrappers over system calls: descriptor passing, metrics | no |
| `agentd` (binary) | `agentd/bin` | wiring and the command line (`serve`, `token`, `migrate`, `gui-launch`, `agent-launch`, `backup`, `backup-verify`, `restore`, `runner`, `watch`, `--diagnose`) | no |
| `agentd-e2e` | `agentd/e2e` | end-to-end and contract tests against a real core process | no |

`core-client` and `controllers` declare `#![deny(clippy::unwrap_used)]`. The whole workspace sets `unsafe_code = "forbid"`; the only exception is `agent-shell/ui/src/hardening.rs`.

## 2. `agent-shell/ui/src`

On non-Linux systems the crate builds as an empty skeleton; everything GTK-specific is under `cfg(target_os = "linux")`.

| Module group | What it does |
|---|---|
| `main.rs`, `app.rs` | entry point; `--smoke` shows the window, waits for the first frame and exits 0 (1 after 10 s) |
| `shell*.rs` | the window: bar and four columns; sizes, access, inner wiring |
| `theme_tokens.rs`, `theme_sync.rs` | colour tokens of both themes and CSS from them; theme exchange with the core (cache in `ui.json`, the core wins) |
| `prefs_io.rs` | atomic read and write of `ui.json` (temporary file and rename, modes `0700`/`0600`) |
| `hardening.rs` | process dumpability off, portals off |
| `a11y.rs` | names and roles of regions for the accessibility tree |
| `core_worker.rs`, `*_worker.rs` | worker threads for core calls and the bridge of events into the GTK main loop |
| `bar_*` | bar feed (bridge from core events to the bar controllers), rig, view, input |
| `tree_*` | session tree widgets, rows, input, actions |
| `chat_*` | chat items, Markdown rendering, streaming apply, widgets |
| `notif_*` | notification cards, toast, widgets |
| `tools_*`, `browser_*`, `terminal_pane.rs`, `ss_*` | tools column: browser pane (WebKitGTK), terminal pane (VTE), session settings, tasks, permissions |
| `settings_*` | Settings dialog widgets, rows, fields, workers |
| `modules_feed.rs` | module panels feed |
| `conn_feed.rs` | connection state banner |
| `dialog_host.rs`, `dialog_form_view.rs` | modal dialogs layer |

Data flow: a frame or widget signal becomes a message to a controller, then an `Effect`, then the bus or a worker thread, then a widget update.

## 3. `agent-shell/controllers/src`

| Files | What |
|---|---|
| `controller.rs`, `bus.rs` | controller contract, `CoreCommand`, `Effect`; synchronous `EventBus` and `BusEvent` (not the core's WebSocket topics) |
| `controllers/*.rs` | one file per controller: `bar`, `browser`, `chat`, `connection`, `dialog`, `indicator`, `metrics`, `modules`, `notifications`, `quota`, `search`, `session_settings`, `session_tree`, `settings`, `shell`, `shortcut`, plus `mod.rs` |
| `bar_cluster.rs`, `bar_types.rs` | bar, metrics and quota controllers together; `bar.*` frames as controllers see them |
| `layout.rs`, `prefs.rs`, `types.rs` | column layout (pure function of width), `ui.json` format, shared values |
| `level.rs`, `forecast.rs` | colour levels and thresholds (half-up rounding), "runs out in" forecast |
| `clock_fmt.rs`, `timezone.rs`, `time.rs` | clock and date, time zone from TZif |
| `chat_*.rs`, `markdown.rs` | chat view models: bubbles, cards, chips, input, stream, attachments, observer cards, limited Markdown |
| `tree_*.rs` | session tree model, navigation, forms, rendering |
| `notif_*.rs` | notification model, rendering, types |
| `settings_*.rs` | Settings pages, forms, fields, specs, save/parse/reply handling, permissions matrix |
| `indicators/` | indicator catalog, collapse, quiet hours, model |
| `module_*.rs`, `manifest.rs` | module manifest and placement |
| `browser_*.rs`, `terminal_view.rs`, `tool_types.rs`, `ss_view.rs` | tools column models |
| `permissions.rs`, `secrets.rs`, `hits.rs`, `keys.rs`, `dialog_form.rs`, `form_types.rs` | permission cards, write-only secrets, search hits, shortcuts, forms |
| `i18n.rs`, `text.rs`, `testkit.rs` | Fluent keys with `assets/i18n/en.ftl`; escaping for Pango; hostile test strings (feature `testkit`) |

## 4. `agent-shell/core-client/src`

`api/` (contract types), `client.rs` and `http.rs` (typed HTTP calls), `blocking.rs` (synchronous wrapper for worker threads), `core.rs` (`CoreClient`: its own tokio runtime in a thread, `CoreEvent` channels out), `supervisor.rs` and `session.rs` (outer loop: health, connect, states, reconnect, subscriptions, `seq`, `ping`/`pong`), `state.rs` (`ConnState`, `ReconnectPolicy`), `queue.rs` (outgoing queue with `client_id`), `token.rs` (memory-only token, `Debug` never prints it; a test-file source exists only with the `test-token-file` feature), `transport.rs` (`Endpoint`: unix socket or TCP), `ws.rs`, `chat.rs`, `tree.rs`, `events.rs`, `settings.rs`, `tools.rs`, `logging.rs`, `error.rs`.

## 5. `agent-web/src`

`main.rs`, `app.rs` (shell), `core.rs` and `api.rs` (browser transport over `fetch` and WebSocket), `view_*.rs` (bar, rail, tree, chat, notes, settings, tools), `outbox.rs` (outgoing queue), `md_view.rs` (Markdown as text nodes), `indicators.rs`, `settings.rs`, `term.rs` (agent terminal viewer), `attach.rs`, `notes.rs`, `design.rs`, `icons.rs`, `util.rs`. Build config: `Trunk.toml`, `index.html`, `style.css`; scripts in `agent-web/scripts/`; Playwright specs in `agent-web/tests/e2e/specs`.

## 6. `agentd`

| Crate | Notable contents |
|---|---|
| `agentd/core/src` | `respond/` (agent turn: prompt, stream, dispatch), `requests/` (permission requests and decisions), `perms.rs`, `rules.rs`, `gate.rs`, `observer/`, `gateway/` (routing, phishing checks), `channels/`, `mcp/`, `browser/`, `events/`, `indicators/`, `search/`, `memory*.rs`, `tasks/`, `triggers/`, `tunnels.rs`, `modules.rs`, `quota.rs`, `secrets.rs`, `shell*.rs`, `export.rs`, `files/`, `tree/`, `watch/` |
| `agentd/db/src` | `store.rs`, `migrate.rs`, `chat/`, `tree/`, `events/`, `perms/`, `tasks/`, `search/`, `backup.rs`, `snapshot.rs`, `integrity.rs`, `retention.rs`, `quota.rs`, `settings.rs`, `modules.rs`, `tunnels.rs`, `indicators.rs`, `memory.rs` |
| `agentd/api/src` | `server.rs`, `routes.rs`, `layers.rs` and `access.rs` (authentication), `limiter.rs`, `hub.rs`, `topics.rs`, `ws.rs`, `health.rs`, `web.rs` (static browser client and security headers), `tree/`, `chat.rs`, `permissions.rs`, `settings.rs`, `tokens.rs`, `secrets.rs`, `tasks.rs`, `tunnels.rs`, `modules.rs`, `search.rs`, `export.rs`, `files.rs`, `metrics.rs` |
| `agentd/bin/src` | `cli.rs` (flags and environment), `serve.rs` and `serve/`, `launch.rs` and `shell_tokens.rs` (GUI and agent token issue), `admin.rs` (token CLI), `runner*.rs`, `watch*.rs`, `backup.rs`, `restore.rs`, `verify.rs`, `diagnose.rs`, `heartbeat.rs`, `dbus_notify.rs`, `web_cfg.rs` |
| `agentd/providers/src` | `anthropic.rs`, `openai.rs`, `sse.rs`, `limits.rs`, `mcp.rs`, `channels/`, `weather.rs`, `scripted.rs` |
| `agentd/packaging` | `.deb` build script, systemd units (user and system), sysusers and sysctl snippets, installers and verifiers for the runner and observer services |

Migrations: `agentd/migrations/0001_init.sql` through `0010_permissions_mcp.sql` (the authoritative schema, see [db.md](db.md)). `fake-agentd/migrations/` holds the subset the mock needs.

## 7. `fake-agentd/src`

HTTP handlers and hub (`handlers.rs`, `hub.rs`), authentication (`auth.rs`), scripted agent (`agent_sim.rs`), chat, bar, events, indicators and tools simulation (`chat_*`, `bar*.rs`, `events_*`, `indicators.rs`), `admin/` (accounts, personas, settings, secrets, modules, tunnels, files, failure injection), `db/` (migrations, snapshots, storage), `clock.rs` (manual and real clocks), `config.rs`, `main.rs`. Scenarios in `fake-agentd/scenarios/*.toml`, JSON schemas in `fake-agentd/schemas/`.

## 8. Configuration, assets, migrations

| What | Where |
|---|---|
| Interface strings | `assets/i18n/en.ftl` |
| Layout and theme cache | `$XDG_CONFIG_HOME/agent-shell/ui.json` |
| Toolchain | `rust-toolchain.toml` (Rust 1.99.1) |
| Dependency policy | `deny.toml` |
| Lint policy | `clippy.toml`, `[workspace.lints]` in `Cargo.toml` |

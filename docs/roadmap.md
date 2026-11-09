# Roadmap

Status: pre-alpha, version 0.0.1-alpha. This page separates what exists in the repository from what does not. It makes no promises about dates.

## What exists

**Core `agentd`** (Rust, SQLite, single writer):

- HTTP and WebSocket API over a unix socket and optional loopback TCP; tokens with scopes and lifecycle; request ids; `If-Match` concurrency; contract in [api.md](reference/api.md) and the OpenAPI draft.
- Sessions, folders, personas, accounts; chat with streaming, idempotent sends, stop; Anthropic and OpenAI-compatible providers; usage and quota accounting.
- Permission engine (allow / ask / deny, three levels, first decision wins, TTL, fail-closed) with an action log; observer as a separate process.
- Events feed, indicators, gateway routing, messenger channel adapters, MCP client, scheduler with cron tasks and triggers, tunnels settings, modules registry.
- Search and memory with a local source and an external MCP memory server ([ADR-0009](adr/0009-memory-via-mcp.md)); a vector index is not enabled.
- Agent runner (isolated command execution), GUI token issue by descriptor, encrypted backup, verify and restore, diagnostics, a `.deb` packaging script with systemd units.

**Native client `agent-shell`** (GTK4 + libadwaita, Linux): the window with bar and four columns, themes, the bar (clock, metrics, quotas, weather, indicators), session tree, chat with permission cards, notifications, tools column (browser, terminal, session settings), Settings pages, search palette. The controllers behind them are shared with the browser client.

**Browser client `agent-web`** (Rust to WebAssembly): sign-in by token, bar, session tree, chat with streaming, permission and observer cards, notifications, Settings, tasks, search, agent terminal viewer, responsive layout for phones. Playwright scenarios exist for chromium, firefox and webkit.

**Tooling:** `fake-agentd` mock with scenarios, contract tests shared with the real core, UI scenarios under headless Wayland, mutation testing setup, `deny.toml` policy, `scripts/check.sh`.

## What does not exist or is incomplete

- **Module sandbox.** Module panels are specified ([ADR-0004](adr/0004-modules-sandbox.md)) but the WebKit sandbox for them is not implemented.
- **Full acceptance on a real stack.** The native shell was not validated end to end under the target greeter and kiosk compositor on the target OS release; development runs on an earlier Ubuntu release.
- **Accessibility.** Automated checks pass (names, contrast, tree roles), but manual screen-reader and Safari-on-device runs are pending.
- **Browser client gaps.** Some e2e scenarios depend on a specific test stand; full acceptance of the browser client is open. The in-page browser panel and a few persona/folder controls are not done.
- **Models.** Providers are connected by API key only; there is no subscription login.
- **Vector search.** Disabled until a policy decision on the required `unsafe` exception and an embeddings provider.
- **Distribution.** No signed package repository, no update channel, no reproducible-build verification; install the `.deb` by hand.
- **Localisation.** The interface is English only.
- **System shell functions** (Wi-Fi, Bluetooth, screen lock, monitors, printers, disks), first-run wizard, module marketplace, mobile client, profile export/import and PAM password change are out of scope for the first release.
- **Observer quality.** Injection detection with a real local model is not measured.

## Direction

1. Close the gaps above in this order: module sandbox, acceptance of the browser client against a real core, manual accessibility pass, validation under the target kiosk session.
2. Stabilize the HTTP/WebSocket contract and publish it as versioned.
3. Packaging and signed distribution.
4. Then: vector memory, additional providers and channels, more languages.

Architecture decisions are recorded as ADRs; see the [index](index.md#decisions-adr). The mock server and the contract tests are the compatibility reference for third-party clients.

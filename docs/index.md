# Documentation

Agent OS GUI is a thin client and a core daemon for an agent-driven desktop: a fullscreen GTK4 shell (bar and four columns: notifications, sessions, chat, tools), a browser client, and the Rust core `agentd` that owns all data. Status: pre-alpha, 0.0.1-alpha, licensed under Apache-2.0 ([ADR-0011](adr/0011-project-license.md)).

## Start here

| Page | What it answers |
|---|---|
| [Getting started](getting-started.md) | prerequisites, building, running the mock and the real core, the browser client, connecting |
| [Architecture](architecture.md) | layers, controllers, the core, data flows |
| [Testing](testing.md) | test levels, commands, the mock core, what is covered |
| [Roadmap](roadmap.md) | what exists, what does not |

## Reference

| Page | Content |
|---|---|
| [API contract](reference/api.md) | HTTP and WebSocket contract between clients and `agentd` |
| [OpenAPI draft](reference/openapi-draft.yaml) | machine-readable description of the same contract |
| [Database](reference/db.md) | `sessions.db` schema, migrations, backup, retention |
| [Code map](reference/code-map.md) | crates and what lives where |
| [Security model](reference/security-model.md) | assets, trust boundaries, threats, residual risks |
| [Security policy](../SECURITY.md) | how to report a vulnerability |

## Decisions (ADR)

Client and product:

| ADR | Topic |
|---|---|
| [0001](adr/0001-init-stack.md) | GUI stack: Rust, GTK4 + libadwaita, WebKitGTK, VTE, kiosk compositor |
| [0002](adr/0002-db-single-writer.md) | `agentd` is the only database writer |
| [0003](adr/0003-api-auth.md) | API authentication: tokens, scopes, GUI token by descriptor |
| [0004](adr/0004-modules-sandbox.md) | module threat model and sandbox |
| [0005](adr/0005-permissions-semantics.md) | permission semantics: levels, ask, first decision wins |
| [0006](adr/0006-kiosk-safety.md) | kiosk session safety and startup resilience |
| [0007](adr/0007-distribution-and-backup-keys.md) | package distribution, backup and key scheme |
| [0008](adr/0008-open-questions-closed.md) | open questions of the first release, closed |
| [0009](adr/0009-memory-via-mcp.md) | memory and search through an external MCP server (`memory_mcp`) |
| [0010](adr/0010-web-client.md) | browser client `agent-web` |
| [0011](adr/0011-project-license.md) | project license: Apache-2.0 |

Core `agentd`:

| ADR | Topic |
|---|---|
| [A-0001](adr/agentd-A-0001-stack.md) | stack: Rust, tokio, axum, rusqlite, rmcp |
| [A-0002](adr/agentd-A-0002-protocol.md) | protocol: REST + WebSocket |
| [A-0003](adr/agentd-A-0003-providers.md) | model providers: a thin layer with two adapters |
| [A-0004](adr/agentd-A-0004-scheduler-in-process.md) | scheduler inside `agentd` |
| [A-0005](adr/agentd-A-0005-permissions-engine.md) | permissions engine |
| [A-0006](adr/agentd-A-0006-runner-packaging.md) | agent runner as a root service, packaging |

## Component documentation

- Browser client build, run and end-to-end tests: [agent-web/README.md](../agent-web/README.md).
- Dependency and license policy: [deny.toml](../deny.toml).

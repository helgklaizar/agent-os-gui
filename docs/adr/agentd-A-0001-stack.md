# agentd A-0001. Stack: Rust, tokio, axum, rusqlite, rmcp

Status: accepted

## Context

The `agentd` core lives in the same Cargo workspace as the shell (`agent-shell/`) and the mock core (`fake-agentd/`); see [architecture](../architecture.md). [ADR-0001](0001-init-stack.md) already fixes Rust, `tokio`, `hyper` 1.x for the HTTP client and `tokio-tungstenite` for WebSocket. The dependency policy keeps the allowed stack closed: a new product layer (DB client, HTTP library, provider) needs its own ADR. The policy was written for the GUI and the mock core and excluded `sqlx`, `rmcp` and a web framework; for the core that restriction is revisited here.

Facts behind the choice:

- Storage is SQLite with a single writer ([ADR-0002](0002-db-single-writer.md)), not Postgres.
- `rmcp` is the official Rust MCP SDK; `sqlite-vec` is pre-1.0.
- Dependency requirements: `rustls` only (`openssl-sys` and `native-tls` are banned), licenses checked by `cargo deny`, one async runtime, one HTTP client, versions pinned in `[workspace.dependencies]`, builds with `--locked`.
- The API has roughly 50 paths in the OpenAPI draft ([api](../reference/api.md)) plus a WebSocket with ten topics; one router serves both a unix socket and TCP, with `SO_PEERCRED` on the socket.

## Decision

| Layer | Choice | Why |
|---|---|---|
| Language | Rust stable, edition 2024, toolchain from `rust-toolchain.toml` | already accepted |
| Async | `tokio` (multi-threaded), `tokio-util` (`CancellationToken`) | the only runtime |
| HTTP and WS server | `axum` on `hyper` 1.x and `tower` | see below |
| HTTP client (providers, outbound) | `hyper` + `hyper-util` + `rustls` | same stack as the GUI client; no second HTTP client |
| Database | `rusqlite` with system SQLite (no `bundled`), own migration runner | see below |
| MCP client | `rmcp`, hidden behind a trait in the `mcp` crate | do not write our own |
| Serialization, time, tokens, errors, logs | `serde`, `serde_json`, `chrono`, `sha2`, `subtle`, `rand`, `uuid`, `thiserror`, `tracing` | already allowed |

### axum rather than bare hyper

- Dozens of routes, parameter extraction, WebSocket and `tower` layers: with bare `hyper` we would write our own router, extractors and auth layer, which add no product value.
- `axum` sits on the `hyper` 1.x and `tokio` already allowed, so there is no second HTTP stack; its WebSocket support uses `tokio-tungstenite`, already in the policy.
- One `Router` serves both listeners (unix socket and TCP); authentication and errors are layers.
- Unix-socket support in `axum::serve` depends on the crate version. The fallback is a custom accept loop on `hyper-util`, which is also needed to read `SO_PEERCRED` through tokio's `UnixStream::peer_cred`. Which scheme is used is decided in the first core milestone; not verified against the locked versions.
- The "no web framework" restriction applies to `agent-shell`; this ADR lifts it for `agentd`.

### rusqlite rather than sqlx

- `sqlx` is banned everywhere including CI; its pool and async interface matter when there are several writers, and here there is one (a dedicated thread) with reads going through `spawn_blocking`.
- `rusqlite` is already used by the mock core, so schema, migrations and queries are shared without a second client.
- The migration runner is our own (`NNNN_name.sql`, a `schema_migrations` table, `PRAGMA user_version` owned by the runner only; see [db](../reference/db.md)); off-the-shelf migrators do not give this behavior.
- Required SQLite features (WAL, `WITHOUT ROWID`, partial indexes, `json_valid`, triggers, `VACUUM INTO`) and the system SQLite version on the target Ubuntu 26.04 are not verified; checking them is the first core task.
- `sqlite-vec` is not linked into `agentd`: the search index is owned by a separate component ([ADR-0002](0002-db-single-writer.md)).

### rmcp

- Used as an MCP client (stdio and Streamable HTTP). `agentd` is not planned as an MCP server.
- Risk: the SDK targets a moving specification. The version is pinned and all calls are isolated in the `mcp` crate behind an `McpClient` trait, so replacing it touches nothing else.
- If the HTTP transport pulls in its own HTTP client (for example `reqwest`), that would be a second HTTP client in the tree. Not verified; it would need an explicit policy exception and a `cargo deny` `[bans]` entry, or the feature stays off and only stdio is used until decided.
- Client behavior is verified in the first milestone; the memory MCP server (`memory_mcp`) is described in [ADR-0009](0009-memory-via-mcp.md).

### Policy changes

This record is the basis for new rows in the dependency policy: `axum` (API server), `rmcp` (MCP client), `rusqlite` for `agentd`, and the note that `sqlx` stays banned. Other candidates (safe wrappers for `SCM_RIGHTS`, `memfd`, `sd_notify`, cron parsing, a D-Bus crate) are not chosen and each must pass the policy criteria separately.

## Consequences

- One core codebase on the same toolchain and guards as the GUI (`cargo fmt`, `clippy -D warnings`, `cargo test --locked`, `cargo deny check`).
- `axum` makes the [api](../reference/api.md) contract implementable without a custom HTTP framework, at the cost of one more dependency and its update cycle.
- All DB writes go through one thread; throughput is bounded by it and will be measured after the first core milestone.
- `rmcp` is the main external risk of the stack, reduced by the trait and the pinned version.
- `rusqlite` is synchronous: any reader call from async code goes through `spawn_blocking`, otherwise it blocks the runtime.

## Alternatives

- **Bare `hyper`.** Rejected as costly with no product value; remains the fallback for the accept loop.
- **`actix-web`, `warp`, `poem`.** Not examined in detail: they would be a second stack next to the already accepted `hyper` and `tokio-tungstenite`.
- **`sqlx`.** Rejected by policy and by the single-writer model.
- **ORMs (`diesel`).** Banned by policy.
- **Own MCP client.** Rejected: expensive and changes with the specification.
- **Claude Agent SDK as the core.** Rejected: it launches the Claude Code binary, is tied to one vendor, and forbids claude.ai login in third-party products. Acceptable only as an optional "code" module.
- **Sidecar `opencode serve` instead of an own loop.** A measurement was proposed but not carried out. This ADR accepts a thin own agent loop in Rust.

# ADR-0009. Agent memory and search through an external MCP server (optional)

Status: accepted as an option (enabled by a setting, does not replace built-in memory)

Date: 2026-11-02

## Context

The spec and the GUI requirements call for memory and one search across sessions, messages, mail, files and memory (Ctrl+K). The built-in plan is a separate `index.db` (SQLite + `sqlite-vec`) owned by an `agent-index` component, plus `memory_facts` in `sessions.db` ([DB reference](../reference/db.md)). An indexer, embeddings and semantic search are a multi-day project of their own, and may duplicate what an external knowledge service already provides.

Assumptions about the external server, here called `memory_mcp`:

- It exposes MCP over Streamable HTTP and offers read-only search tools (`search`, `doc_search`) under a scope-limited token; full-text search works, semantic search depends on its embedding provider being configured.
- It accepts only `Host` values `localhost`, `127.0.0.1` and `::1`, so access from a user machine goes through an SSH tunnel. Tokens have ranks (`ro` < `agent` < `owner`) and the tool list depends on the token scope.
- It typically runs on a shared host, not on every user machine.

## Decision

1. **Memory and search source is an option.** `agentd` connects to the `memory_mcp` MCP server as to any MCP server: `search`, `doc_search` and similar calls go through the core's MCP client, not from the GUI. The GUI does not know about `memory_mcp`: `SearchController` only calls `GET /search` on `agentd`, and results arrive with a group and source label as data.
2. **The built-in `index.db` stays as fallback and default.** While `memory_mcp` is unavailable, not configured or has no token, search uses the local index. Switching is the setting `memory.source` (`local` | `memory_mcp` | `both`), default `local`, stored in the `settings` table and changed through `PATCH` on settings. With `both`, results are merged and each hit carries a `source` field (`local` | `memory_mcp`) in the `GET /search` response ([API reference](../reference/api.md)).
3. **Token and network.** The `memory_mcp` token is created by the owner, stored through `PUT /secrets/{ref}` ([ADR-0003](0003-api-auth.md)), never read back and never logged. Scope is `ro` (read); writing to `memory_mcp` through this option is not provided. The connection is an SSH tunnel or loopback; direct access with a foreign `Host` is rejected by the server.
4. **Data.** Only the search query is sent to `memory_mcp`; user messages, mail and files from `sessions.db` are not. Results are not copied into `sessions.db` without an explicit action: only a reference and a short excerpt are kept in the query response.
5. **Permissions.** An agent call to `memory_mcp` is a "read"-level action under [ADR-0005](0005-permissions-semantics.md). Unavailability of `memory_mcp` does not block chat; search degrades to the local index.
6. **Contract.** `fake-agentd` emulates the `memory_mcp` source with a fixture; a real MCP server is not required for acceptance.

## Conditions for enabling the option

- Semantic search on the server actually returns results, not only full-text search; until then the option provides text search only.
- Agreement on which data the server exposes to a user's core (no personal data of other people) and on an `ro` token.
- The network path works without a manual step (tunnel autostart or another way); otherwise the user perceives "search silently stopped finding things".
- Latency and unavailability are measured; the threshold for falling back to the local index is an assumption: 2 s per request, 3 consecutive failures (to be replaced by measurement).

## Consequences

- The built-in `index.db` and `memory_facts` stay as planned; a `memory_mcp` source and a setting are added.
- `index.db` could be dropped later if `memory_mcp` becomes a mandatory dependency; this is not decided here.
- Search gains a dependency on an external service: `--diagnose` needs a line "memory source: local / memory_mcp (available/not)", and the risk register needs an entry for this dependency.
- Before use, check that the server's MCP API has not changed: tools and schemas evolve, and `tools/list` depends on token scope.

## Alternatives

- **Built-in `index.db` only.** Remains the default mode; the cost is a separate indexer and embeddings.
- **`memory_mcp` only.** Rejected: depends on network and a shared host; search stops working on a machine without access; semantic search may not be ready.
- **Copying `memory_mcp` data into the local index.** Rejected: duplicates storage of foreign data, including personal data, on the user machine.
- **Direct GUI access to `memory_mcp`.** Rejected: violates the principle that the GUI knows only `agentd`.

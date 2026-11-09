# agentd A-0005. Permissions engine: implementing ADR-0005

Status: accepted

## Context

[ADR-0005](0005-permissions-semantics.md) defines the semantics: the effective permission is the strictest of three levels (`deny` > `ask` > `allow`); a lower level may only tighten; an attempt to loosen yields `403 permission_denied` and an `observer_findings` row (`kind = permission_loosen`); "Always" creates a persona-level or global rule; decisions are first-wins (`409`); request TTL is 10 minutes; the client keeps an outbound queue; the system fails closed when the observer is unavailable. [ADR-0002](0002-db-single-writer.md): permission checks, `actions_log` and events pass through a single write point. [ADR-0003](0003-api-auth.md): only the local GUI with `admin` on the socket can decide a request; text from external sources is untrusted. Tables: see [db](../reference/db.md). Actions: `read`, `send`, `delete`, `pay`, `shell`, `browse`, `memory_write`, `config_write` ([api](../reference/api.md), `Permission` schema).

## Decision

### Division of responsibility

- **Pure rules in `core`:** the effective-mode function (maximum strictness across the three levels; a missing level does not participate), the "does this write loosen" check, and classification of actions into "dangerous" (deletion, payments, outbound sending, terminal commands from a danger list, config and permission changes) and "safe reads". No I/O; tested with tables.
- **`perms` engine:** request state, TTL, waiting for a decision, interaction with the observer.
- **`db` writer:** writes rules and the journal. The "tighten only" check is **repeated inside the transaction** that writes `permissions` (same pure function), so no other code path can bypass it; the `403` and the finding are produced in one transaction.

### Decision for an action

For an agent, module, or cron action the engine computes:

| Effective permission | Result |
|---|---|
| `allow` | the observer checks dangerous categories; safe reads proceed immediately; an `actions_log` row is written |
| `ask` | a request is created and the agent turn waits for a decision |
| `deny` | the action does not run, `403 permission_denied` (for the agent, tool result `denied`), a journal row, no request |
| observer unavailable, action dangerous | **fail closed:** the action does not run; the request returns as `ask` with a "check unavailable" note in `detail` until the observer returns or the user explicitly confirms ([ADR-0005](0005-permissions-semantics.md)) |

Observer unavailability is a request timeout or no connection (timeout value undefined). Observer state is journaled and shown by a bar indicator. Unavailability of the memory MCP server (`memory_mcp`) does not block chat ([ADR-0009](0009-memory-via-mcp.md)).

### Request and its states

- States: `pending`, `allowed`, `denied`, `expired` (diagram in [architecture](../architecture.md)). The request id equals `actions_log.id`.
- Storage: a row is created when the request arises; the chat card is a `role = system` message with `meta.kind = "permission_request"`. The `status`, `decided_by`, `decided_at`, `expires_at` columns do not exist in [db](../reference/db.md). Of two options the second is proposed:
  1. add columns to `actions_log` (changes a migration not yet written);
  2. keep current state in engine memory and write the outcome (`decision`, `result` with reason `expired`, the card's `meta_json`) to `actions_log` and `messages` without a schema change, at the cost of being unable to restore `pending` after a restart (all `pending` from the previous run are closed as `expired`).
- Waiting for a decision is an `await` on a one-shot channel with a TTL timer (10 minutes, configurable in Settings, Permissions). Cancellation on session `stop`, session deletion (resolved as `deny`) or core restart; behavior on `stop` is undefined, proposal: the request stays `pending` until TTL.
- The decision is made by `POST /permission-requests/{id}/decision` with `admin` on the local socket. "First wins" is a conditional update (`UPDATE ... WHERE status = pending`, as in [db](../reference/db.md)): the loser gets `409` with `details.reason = "already_decided"`, and WebSocket emits `permission.resolved` with `decided_by`.
- `always`: after an explicit level choice it creates or changes a persona or global rule (`PUT /permissions/{scope}/{scope_id}/{action}`); `scope: "session"` gives `422`; if the write would loosen a stricter level the answer is `403` (request state in that case is undefined, proposal: stays `pending`).
- An external token (`external`, at most `chat`) and `agent` and `module` tokens can never decide requests or change permissions; `x-local-only` routes are rejected over TCP.

### Observer

- Channel `agentd` to `agent-watch` (proposal): the daemon's unix socket, a "check this action or incoming text" request and a "verdict" response (`blocked`, `asked`, `allowed`, with `kind`: `injection`, `phishing`, `dangerous_cmd`, `module`); a timeout is mandatory. `agentd`, not the observer, writes the finding (`observer_findings`) so the DB writer stays unique. The protocol format is not yet defined.
- Checked: incoming mail and chats (injection, phishing leading to isolation), dangerous commands (`rm -rf` leads to stop and confirmation), module installation (signature, permissions; [ADR-0004](0004-modules-sandbox.md)).
- The observer runs on an isolated local model and sees no keys; unit restrictions are a packaging task, not verified.

### Request source and untrusted input

Each request carries its source (session, cron, module, observer) and, if any, the untrusted fragment that provoked it (an email, a page). These are shown in the card ([ADR-0003](0003-api-auth.md), trust boundary). Text from external sources cannot itself grant permissions, change persona rules, or request tokens.

### Journal

Every action, request and decision is an `actions_log` row (`actor`: `agent`, `cron`, `user`, `observer`, `module`); `args_json` carries no secrets and is masked, and is cleared after 90 days and on session deletion. Token issue and revocation are journal rows too. Undo is not promised in 0.1; `undo_json` stays as a future field.

### Tests

Table tests of the pure rules (all combinations of three levels), tests that loosening is impossible through any route, a race of two decisions, TTL with a controllable `Clock`, fail-closed with the observer stopped, and secret leakage into the card. The `ask_always_persists_permission` and `shell_permission_ask_roundtrip` tests from [ADR-0005](0005-permissions-semantics.md) move to the real engine.

## Consequences

- One implementation of the rules for the GUI, phone and cron: all go through one engine and one writer.
- The double "tighten only" check (in `perms` and in the write transaction) adds code but closes bypass around `perms`.
- Fail-closed can halt work when the observer fails (accepted in ADR-0005); manual confirmation in the card softens this.
- Requests without a session (cron, module installation) have no chat card; their visibility in the GUI depends on the event feed.
- [db](../reference/db.md) and [api](../reference/api.md) must be reconciled on request storage.

## Alternatives

- **A separate process for permission checks.** Not needed: the single writer is already in `agentd`, and an extra process lengthens every action. The observer is the exception, being separate for other reasons.
- **Checks in API handlers (no engine).** Rejected: agent turns and cron do not go through the API, giving two implementations.
- **A separate requests table.** The API document deliberately has none; remains possible if the `actions_log` columns prove insufficient.
- **"Lower level overrides", "ignore silently", fail-open, last-wins.** Rejected by ADR-0005 itself.

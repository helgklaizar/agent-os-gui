# ADR-0005. Permission semantics

Status: accepted

## Context

Permissions are set at three levels: global, persona, session (see [db.md](../reference/db.md)). The original spec said "the lower level overrides", while tests and the API contract said "the lower level may only tighten", and the DB doc required "ignore and write to `observer_findings`" while the API answered `403`. The "Always" button on an `ask` request card was supposed to stop later prompts, which is impossible under "strictest of three": a session-level `allow` does not lift a global `ask`.

A security audit found the model internally inconsistent: "Always" did nothing, the outbound queue replayed on stale state, the decision race between GUI and phone was undefined, and behavior with an unavailable observer was undefined. This ADR fixes the semantics.

## Decision

### Effective permission

The effective permission of an action is the strictest of the three levels: `deny` is stricter than `ask`, `ask` stricter than `allow`. A missing level does not participate. A lower level (session, persona) can only tighten the global rule and the rule of the level above it.

### Attempt to loosen

A request that tries to loosen a rule (a session sets `allow` under a global `ask` or `deny`) gets `403 permission_denied` from the core. The rule is not applied and `observer_findings` gets a row (`kind = permission_loosen`, level, action, requester). The GUI shows the error in the dialog instead of staying silent.

### "Always"

"Always" does not loosen or change the session level. It creates or changes a rule at a level the user picks: persona or global. The request card shows a level selector ("this persona only" or "globally") and the action with arguments; the rule is created through `PUT /permissions` with an `admin` token ([ADR-0003](0003-api-auth.md)) after explicit confirmation. If a stricter level has `ask` or `deny`, "Always" is unavailable and the card explains which level blocks it. The rule is written to `actions_log`; `undo_json` is reserved for the future and rollback is not promised in 0.1.

### Decision on a request: first wins

A permission request is `pending`, then `allowed`, `denied` or `expired`. The local GUI decides with an `admin` token over the socket. External clients (phone, CLI) hold a token no higher than `chat` and cannot decide; a browser client with a `web` token of scope `decide` may make one-time `allow`/`deny` decisions but never "Always" ([ADR-0003](0003-api-auth.md)). When several clients can decide (two GUI windows), the first accepted answer wins; later ones get `409 conflict` with the current status. Clients synchronize through a WS event on request change. `GET /permission-requests/{id}` is required.

### TTL and the outbound queue

- An `ask` request expires after a timeout (10 minutes by default, configurable in Settings, Permissions); an expired request is `denied` with reason `expired`, and the agent action is not performed.
- The GUI's outbound input queue during connection loss is bounded by TTL (5 minutes by default) and item count. Before replay the client shows the accumulated list and lets the user cancel; only items with current state are replayed (the session exists, permissions unchanged).
- If permissions or the session changed while the client was offline, the item is dropped with a message. A decision on an `ask` from the queue is never replayed; it must be made again.

### Unavailable observer: fail-closed

If `agent-watch` is unavailable, "dangerous" categories (deletion, payments, outward sending, dangerous terminal commands, config and permission changes) are blocked until the observer returns or the user explicitly confirms the action in the card. Safe reads pass. The observer state is shown in the bar and logged.

### Document alignment

The DB doc and [api.md](../reference/api.md) behave the same way: `403` plus a row in `observer_findings`. The divergence from the original spec ("the lower level overrides") is deliberate.

## Consequences

- Tests for "Always" and the shell `ask` round trip follow this semantics: "Always" changes the persona or global rule, not the session's.
- The API gets the level in the decision body, `GET /permission-requests/{id}`, codes `403` and `409`, status `expired`; `PermissionRequest` gains observer fields (`kind`, `verdict`).
- Several local clients with `admin` need a shared event source for requests (WS), otherwise first-wins is invisible to the client.
- Fail-closed can stop work when the observer fails: an acceptable price for safety, softened by manual confirmation.
- A rule cannot be loosened at a lower level, which limits a session's "autonomous mode": it removes prompts only where upper levels allow.

## Alternatives

- **"Lower overrides" (as in the original spec).** Rejected: a session could allow what a global `deny` forbids, so the agent bypasses the ban through a session.
- **"Ignore silently" without `403`.** Rejected: the user does not see that the rule was not applied.
- **"Always" changes the session-level rule.** Rejected: does not work under strictest-of-three.
- **Last-wins, or locking by whoever opened the request first.** Rejected: causes lost answers and hangs if the client closes.
- **Fail-open with an unavailable observer.** Rejected: an observer failure would disable protection exactly in dangerous cases.

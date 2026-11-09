# Core API: GUI ↔ agentd contract

> Status: **contract draft**. The contract is derived from [architecture.md](../architecture.md), [db.md](db.md), and the decisions in [ADR-0001](../adr/0001-init-stack.md) (stack), [ADR-0002](../adr/0002-db-single-writer.md) (single DB writer), [ADR-0003](../adr/0003-api-auth.md) (authentication), [ADR-0004](../adr/0004-modules-sandbox.md) (module tokens and allowlist), [ADR-0005](../adr/0005-permissions-semantics.md) (permissions), [ADR-0006](../adr/0006-kiosk-safety.md) (core unavailable and the Recovery session), [ADR-0007](../adr/0007-distribution-and-backup-keys.md) (backup keys, versions), [ADR-0008](../adr/0008-open-questions-closed.md) (closed questions Q1–Q10) and [ADR-0009](../adr/0009-memory-via-mcp.md) (memory via the `memory_mcp` MCP server). Topic and route names are working names; **on any mismatch with the implementation this document is corrected**, not the code. Machine-readable form: [openapi-draft.yaml](openapi-draft.yaml) (extensions `x-scope` and `x-local-only`).

The GUI is a thin client: everything it shows comes from the core through this API, and it writes nothing to the DB directly ([db.md](db.md) section 1). Secrets are never returned through the API: not in responses, not over WS, not in logs.

## 1. Transport and authentication

| Channel | Address | Who connects | Authentication |
|---|---|---|---|
| Local (primary) | unix socket `$XDG_RUNTIME_DIR/agentd.sock` | `agent-shell`, CLI on the same machine | **token required** (`Authorization: Bearer <token>`) + client uid check via `SO_PEERCRED` |
| From outside through a tunnel | TCP `127.0.0.1:4096`, then WireGuard / Tailscale / SSH / Cloudflare / ngrok | phone, other clients | the same token, lifetime and revocation, `Origin` and `Host` checked against the tunnel address allowlist |

Decisions: [ADR-0001](../adr/0001-init-stack.md) (fork 3: socket locally, TCP only from outside) and [ADR-0003](../adr/0003-api-auth.md) (tokens and scope). The earlier "socket without a token" scheme is cancelled: a `0600` socket is reachable by any process of the user, including the agent shell.

### 1.1 Tokens and scope

| Scope | What is allowed |
|---|---|
| `read` | reading: sessions, messages, events, quotas, settings (without secrets), subscribing to WS topics |
| `chat` | `read` + sending messages, stopping a stream, creating and editing sessions, folders, threads, session tasks, bookmarks, marking as read, theme |
| `decide` | `chat` + a one-time decision on a permission request (`allow`/`deny`) with a `web` token; "Always" and everything else belongs to `admin` |
| `admin` | `decide` + permissions and decisions on permission requests, `logout`, personas, accounts and their connection, modules, triggers, tunnels, settings pages, secrets, files, issuing and revoking tokens |

- The token of a browser client (`kind = web`, scope `chat` or `decide`, lifetime 1–30 days) is passed over WS as `Sec-WebSocket-Protocol: agentd.v1, bearer.<token>`; the core answers `agentd.v1`.
- `GET /health` and, with `--web-root`, the static `GET /` and `GET /app/*` are anonymous. The root `security` in OpenAPI no longer allows an empty scheme.
- The **GUI token** (`kind = gui`, scope `admin`) **is not stored in a file**: a `0600` file and `/proc/PID/environ` are readable by any process of the same uid, and `SO_PEERCRED` does not distinguish processes of one uid. The core hands the token over when it starts the child GUI process, through an open descriptor (fd) or a socketpair; the GUI keeps it in memory only and writes it nowhere. On a GUI restart a new token is issued and the old one is revoked. Protection rests on isolation: agent commands are executed by the **core** in its own pty under a separate uid `agentos-run` (`systemd-run` isolation), and that process does not see the GUI token.
- **Agent and module tokens** ([ADR-0004](../adr/0004-modules-sandbox.md)): `kind = agent` (scope `chat`, received by the agent process over a separate channel through an fd when the unit starts) and `kind = module` (module token, scope `read` or `chat` per the manifest). Both carry an `allowlist`, a list of allowed methods such as `POST /sessions/{id}/messages`; a request outside the list gets `403 insufficient_scope`. The core issues and revokes them (agent start, module install and removal); `POST /tokens` does not create them, but `GET /tokens` lists them and `DELETE /tokens/{id}` revokes them. They can never decide permission requests or change permissions.
- External clients (phone) get a token through `POST /tokens` (local, `admin`): `kind = external`, scope no higher than `chat` (an external client does not get `admin`; the restriction also exists in the DB: `CHECK (kind <> 'external' OR scope <> 'admin')`). The token value is shown once; the core stores only the SHA-256 in hex ([db.md](db.md) section 3.13).
- **Token format:** an opaque string `agt_` + 32 random bytes in base64url (generator `rand`; the value carries no meaning and does not encode the scope); the DB holds only `sha256(token)` (`sha2`), hashes are compared in constant time (`subtle`); request and event identifiers are UUID v7 (`uuid`). These crates belong to the closed dependency list of the first sprint.
- **A token in the address is forbidden:** `?token=…` (and any parameter carrying a token) is rejected with `401 unauthorized` and `details.reason = "token_in_url"`; a token is accepted only in the `Authorization: Bearer` header (for WS, at upgrade). A WS attempt with a token in the address is closed with code `4401`.
- Lifetime of an external client token: 30 days by default, 90 at most (working values from ADR-0003). Revocation: `DELETE /tokens/{id}`, WS connections using it are closed with code `4401`. Limit of failed attempts over TCP: 5 per minute per address (working value).
- Routes with `x-local-only` (`logout`, `permissions`, decisions on permission requests, `secrets`, `files`, tokens) are rejected over TCP regardless of the token.
- The GUI token lives only in the memory of the GUI process, the external client token lives in its system secret store; neither ends up in logs, the feed or the diagnostics report ([architecture.md](../architecture.md)).

### 1.2 General

- HTTP/1.1 and WebSocket (upgrade on `/v1/ws`) share one socket and one TCP port. The WS token is passed in the `Authorization` header at upgrade (forbidden in the address, section 1.1).
- Content is JSON, UTF-8. Time is `INTEGER` unix seconds in UTC (as in [db.md](db.md)); the client converts to the local zone for display.
- Identifiers are integers (`INTEGER PRIMARY KEY`), except **modules** (string `id`) and **indicators** (a string such as `cpu`, `whapi`, `backup`, `quota.cl1`, where the part after the dot is the account `slug` from [db.md](db.md)). The account primary key (`account_id`) is an integer in the contract; `slug` is a stable short name (`cl1`, `wa1`) needed only for indicator ids.
- **`X-Request-Id`.** The client sets the header `X-Request-Id: <UUID v7>` on every HTTP request and at the `/v1/ws` upgrade. If the header is missing or is not a UUID, the core creates its own. Every response (including errors and `202`) returns the same `X-Request-Id`; in an error body it is duplicated in the field `error.request_id`. The core writes the value to its log, and the `request_id` fields in the GUI and core logs match. The value contains no personal data and is not used as a security identifier (idempotency is `client_id` and the tool `request_id`, not the header).
- Stable cursors and message ordering use `id`, not `ts` (one-second precision does not order messages within a second).

## 2. Versioning

- Path prefix `/v1`. An incompatible change gets a new prefix `/v2`; the old one lives in parallel until clients migrate.
- Compatible changes (new fields, new WS message types, new optional parameters) do not change the version; **the client must ignore unknown fields and unknown `type` values**.
- `GET /v1/health` without authentication (the only anonymous route) returns the fields from section 2.1; every response carries the header `X-Agentd-Version`.
- If `api.major` does not match, the client shows "Core version is not supported" and does not subscribe to topics. The remaining `/health` parsing rules (`min_client`, `min_gui`/`max_gui`, `capabilities`) follow from the fields below.

### 2.1 `GET /v1/health`

```json
{
  "status": "ok",
  "version": "0.3.1",
  "api": { "major": 1, "minor": 4 },
  "min_client": "0.2.0",
  "min_gui": "0.2.0",
  "max_gui": "0.4.x",
  "user_version": 8,
  "capabilities": ["tool-results", "if-match", "ws-seq"],
  "migration": null
}
```

| Field | Type | Meaning |
|---|---|---|
| `status` | `ok` · `starting` · `degraded` · `migrating` | core state; `migrating` means the core is applying DB migrations and mutations are not accepted |
| `version` | string | core version |
| `api` | `{major, minor}` | `major` grows on an incompatible change (new prefix `/vN`), `minor` on compatible additions |
| `min_client` | string | minimum GUI version the core agrees to work with |
| `min_gui`, `max_gui` | strings | range of GUI versions tested with this core; `max_gui` allows a pattern (`0.4.x`) |
| `user_version` | integer or `null` | `PRAGMA user_version` of the database (set only by the migration runner, [db.md](db.md) section 6); `null` until the core has read it |
| `capabilities` | array of strings | enabled capabilities (`tool-results`, `if-match`, `ws-seq`, and also `terminal` for an existing `agent-runner` socket, `observer-model` for an enabled model-based observer, `observer-process` for an available observer running as a separate process); the GUI checks for a feature by it and ignores unknown ones |
| `migration` | `null` or object | `{status, from, to, step?, progress?}`, `status` is `running`, `done` or `failed`; `from`/`to` are `user_version` before and after, `step` is the name of the current migration (`0004_chat`), `progress` is 0..1 |

**With `status = "migrating"`:** any request other than `GET /health` gets `503` with `error.code = "migrating"`, `Retry-After` and `error.details.migration` (the same object). WS is not accepted or is closed with code `1001`. The GUI does not treat this as an error and does not show the "No connection" banner: it shows the status "The core is updating the database" with progress, polls `/health` (working period 2 s) and does not drop the outgoing queue (section 7). With `migration.status = "failed"` the GUI shows "Database update failed" with a "Diagnostics" button and recovery instructions.

## 3. Errors and conditional requests

Single body format for any code ≥ 400:

```json
{ "error": { "code": "not_found", "message": "Session 42 not found", "details": {}, "retryable": false, "request_id": "0192f3a8-7c1e-7d44-9a3b-5e6f1c2d3b4a" } }
```

| HTTP | `code` | When | Retry |
|---|---|---|---|
| 400 | `bad_request` | JSON not parsed, invalid parameter | no |
| 401 | `unauthorized` | missing, wrong, expired or revoked token (on any channel); token in the address (`details.reason = "token_in_url"`) | no |
| 403 | `permission_denied` | action forbidden by policy (`deny`), attempt to loosen a rule at a lower level (a record in `observer_findings`), path outside the allowed set | no |
| 403 | `insufficient_scope` | the token does not cover the operation (`x-scope`) or the route is local only | no |
| 403 | `forbidden_origin` | `Origin` or `Host` did not match over TCP | no |
| 404 | `not_found` | the object does not exist | no |
| 409 | `conflict` | state conflict: moving a folder into itself; **permission request already decided** (`details.reason = "already_decided"`, the first decision wins) | no |
| 412 | `precondition_failed` | `If-Match` did not match the current `rev` (someone else edited) | after re-reading |
| 413 | `payload_too_large` | attachment above the limit (working value 25 MiB, see section 9) | no |
| 422 | `validation_failed` | fields failed validation; `details` is the list of fields | no |
| 428 | `precondition_required` | `PATCH` or `PUT` without `If-Match` where it is mandatory | no |
| 429 | `rate_limited` | requests too frequent | yes, after `Retry-After` |
| 500 | `internal` | core failure | yes, with a pause |
| 503 | `core_down` | the core is starting or a source is unavailable (the code was renamed from `core_unavailable`; the GUI state `core_unavailable` is a separate concept, section 7) | yes, with a pause |
| 503 | `migrating` | the core is applying DB migrations; `Retry-After`, `details.migration` (section 2.1) | yes, after `Retry-After`, without showing an error |

**Confirmation (`ask`) is not an error.** Any mutation whose action is in `ask` mode answers `202 Accepted` with a `PermissionRequest` body; the user decides through `POST /v1/permission-requests/{id}/decision` (section 5.4, [architecture.md](../architecture.md)). Permission is never granted without an explicit user action. Routes that support this are marked `202` in OpenAPI (messages, clear, export, delete, module install, settings, secrets, files, tunnels).

**Conditional requests (settings, bookmarks, files).** `GET` returns `ETag` (= `rev`) and the field `rev` in the body. A change is `PATCH` (JSON merge patch) or a whole `PUT` with a mandatory `If-Match: <rev>`; without the header `428`, with a stale one `412`. This way two clients (GUI and phone) do not overwrite each other: on `412` the client re-reads the page and repeats the edit.

## 4. WebSocket

A browser passes the token in `Sec-WebSocket-Protocol: agentd.v1, bearer.<token>` (see section 1.1).

One endpoint: `GET /v1/ws` (upgrade). Topics are subscribed to after the connection. Subscribing needs a scope of at least `read`.

### 4.1 Client messages

```json
{ "op": "subscribe",   "topic": "session.42" }
{ "op": "unsubscribe", "topic": "session.42" }
{ "op": "ping" }
```

### 4.2 Server message envelope

```json
{ "topic": "bar.sys", "type": "snapshot | delta | event", "seq": 17, "ts": 1793571261, "data": { } }
```

- For every subscription the core first sends a `snapshot` (the full current state), then `delta`/`event`. This implements the principle "after reconnecting, request a snapshot again and then listen to deltas".
- `seq` is a per-topic counter within a connection. A gap (`seq` grew by more than 1) means a loss: the client does `unsubscribe` + `subscribe` and gets a new snapshot.
- The core answers `ping` with `{"type":"pong"}`; the core sends `ping` itself if the silence exceeds the interval (section 7).

### 4.3 Topics

| Topic | Who subscribes | What arrives |
|---|---|---|
| `bar.sys` | MetricsController | hardware metrics, period 2 s |
| `bar.quota` | QuotaController, SettingsController | AI account quotas |
| `bar.weather` | BarController | weather (core or module), period 30 min |
| `bar.indicators` | IndicatorController | states of the extra bar indicators |
| `events` | NotificationsController | notification feed |
| `sessions` | SessionTreeController | tree of folders, sessions and threads, badges |
| `session.<id>` | ChatController, BrowserController, TerminalController, SessionSettingsController | messages, streaming, permission requests, browser commands, agent terminal output and status, session tasks |
| `modules` | ModulesController | module list and placement |
| `settings` | SettingsController | settings changes |
| `connections` | SettingsController | account and connection statuses, OAuth and QR progress |

In the architecture description SettingsController may be named as subscribed to `quota`; in the contract this topic is called `bar.quota`.

### 4.4 Payload

**`bar.sys`**: `snapshot` and `delta` are identical:

```json
{ "cpu": 14, "gpu": 20, "ram": 66, "disk": 38, "temp": 38 }
```

Percentages 0–100, temperature in °C. The client applies the color thresholds (50/80, temperature 60/80).

**`bar.quota`**: `snapshot`: `{ "items": [Quota] }`, `delta`: one changed `Quota`. This is **accounting by fact**, not the remaining subscription: `usage` (tokens and cost per day or session from API responses), `limits` (request and token limits of the API key from the provider rate-limit headers), `load` (local model). The remaining subscription window (5 h) is not returned and not shown, because there is no official API (Q1, [ADR-0008](../adr/0008-open-questions-closed.md)). `Quota.kind = "local"` and `window_h = 0` mean a local model without a limit, in which case `load` (0..1) is filled. If the key limit is not yet known (no response with rate-limit headers yet), `window_h`, `used`, `rate`, `limits` are `null` and the GUI shows only `usage`. `eta_at` (unix seconds when it runs out) is filled **only if the core expects the remainder to run out before `reset_at`**; otherwise `null`, and the client shows "Enough until reset". Color boundaries: remaining > 40% green, > 15% yellow, otherwise red, rounded to integers.

**`bar.weather`**: `snapshot` and `delta` are identical:

```json
{ "temp_c": 21, "icon": "sun", "text": "clear", "place": "Berlin", "updated_at": 1793571261 }
```

The weather source is the core or a module; when it is unavailable the `snapshot` arrives with `null` in the fields and the client does not show a made-up value. The topic and field names are working names.

**`bar.indicators`**: `snapshot`: `{ "items": [IndicatorState] }`, `delta`: one `IndicatorState`. `state`: `ok | warn | bad | nodata | unknown` (`nodata` means no data, gray; `unknown` means the state is not determined yet, for example right after start; as in [db.md](db.md) `indicators.last_state`). The core keeps the state and the transition counter; an event is created **on every transition** (including red → yellow), not by a read flag.

**`events`**: `event` with an `Event` body; `delta` of the form `{ "read": [<id>, …] }` when marked as read (synchronization between clients). `snapshot` is the latest events with the default filter; the rest comes through `GET /v1/events`.

**`sessions`**: `delta` of one of these kinds:
`{ "op": "folder.created|folder.updated|folder.deleted", "folder": Folder | {id} }`,
`{ "op": "session.created|session.updated|session.deleted", "session": Session | {id} }`,
`{ "op": "unread", "session_id": 42, "unread": 3 }`.
`snapshot`: `{ "folders": [Folder], "sessions": [Session] }`.

**`session.<id>`**: `snapshot`: `{ "session": Session, "messages": [Message] (the last page, including messages in status streaming with the accumulated `body_md`), "open_requests": [PermissionRequest], "tasks": [Task] }`. `event` by the field `kind`:

| `kind` | `data` fields | Meaning |
|---|---|---|
| `message.created` | `message: Message` | new message (own from another client, or from the agent) |
| `message.delta` | `message_id`, `offset`, `text` | streaming chunk; `offset` is the number of UTF-8 bytes already in `body_md` before this chunk. The client applies the chunk only if `offset` equals the length of its accumulated text; otherwise it does `unsubscribe` + `subscribe` |
| `message.done` | `message_id`, `status` (`done|stopped|error|interrupted`), `final_len` | end of stream; `final_len` is the final length of `body_md` in bytes, which the client uses to check that nothing was lost |
| `typing` | `on: bool` | "agent is typing" indicator |
| `permission.request` | `request: PermissionRequest` | permission request (`ask`) or observer confirmation |
| `permission.resolved` | `request_id`, `status`, `decided_by`, `result` | request decided (including by another client or by timeout) |
| `browser.command` | `command`, `url`, `request_id` | agent command to the browser (rule `browse`); the GUI executes the command in a separate browser profile and returns the result (DOM, status) through `POST /sessions/{id}/tool-results` (section 5.2) |
| `terminal.command` | `command`, `cwd`, `request_id` | informational: the core **executes** the agent command itself (rule `shell`, observer) in its own pty under uid `agentos-run`; the GUI does not execute the command and does not return a result. Shown in the agent terminal panel. `request_id` is an opaque string issued by the core, it is not a permission request id |
| `terminal.output` | `request_id`, `seq`, `stream` (`stdout|stderr`), `data` | chunk of agent command output from the core (read-only panel, accumulated output limit 256 KiB, then `truncated`); `seq` is the chunk sequence number within `request_id` |
| `terminal.status` | `request_id`, `status` (`started|exited|killed|denied|timeout`), `exit_code?`, `duration_ms?`, `truncated?` (`true` if the output was cut by the limit; present for `exited` and `timeout`) | lifecycle of the agent command from the core; `denied` is a refusal by policy or the observer |
| `task.created` / `task.updated` | `task: Task` | session task created or changed |
| `task.run` | `run: TaskRun` | task run started or finished |
| `session.updated` | `session: Session` | session settings changed |

**`modules`**: `snapshot`: `{ "items": [Module] }`, `delta`: one `Module`.
**`settings`**: `delta`: `{ "page": "personas", "rev": 12 }` (the client re-reads the page `GET /v1/settings/{page}` if it is not the author of the change). The same for `indicators` and `theme`.
**`connections`**: `snapshot`: `{ "items": [Account] }`, `delta`: `{ "account_id": 3, "status": "qr" }` or connection flow progress `{ "account_id": 3, "flow": ConnectFlow }`. Status `qr` means "the number is unlinked, scan the QR" (WhatsApp channel).

## 5. HTTP routes

All under `/v1`. `{id}` is an integer (except modules and secrets). Responses: `200` with an object, `201` on creation, `204` without a body for deletes and actions with no result, `202` when a user decision is needed (section 3). The "scope" column is the minimum token scope ([section 1.1](#11-tokens-and-scope)).

### 5.1 Sessions, folders, threads

| Method and path | scope | Purpose |
|---|---|---|
| `GET /sessions` | read | list (`?folder_id`, `?parent_id`, `?archived`) |
| `POST /sessions` | chat | create: `name`, `folder_id?`, `persona_id?`, `path?` |
| `GET /sessions/{id}` | read | session with `effective_persona_id` |
| `PATCH /sessions/{id}` | chat | `name`, `folder_id`, `persona_id`, `path`, `cfg` (session settings; permissions can only be tightened), `archived` |
| `DELETE /sessions/{id}` | chat | delete the session and its threads (cascade, section 5.9); `202` if the action `delete` is in `ask` mode |
| `POST /sessions/{id}/threads` | chat | create a thread: `name`; inherits the persona, path and context of the parent |
| `POST /sessions/{id}/read` | chat | open the session: reset `unread` and mark the events of this session as read |
| `POST /sessions/{id}/clear` | chat | clear the history; `202` on `ask` |
| `POST /sessions/{id}/export` | chat | export to `~/.agentos/export/`; the response is the path and size; `202` on `ask` |
| `GET /folders` · `POST /folders` | read · chat | list · create: `name`, `parent_id?`, `persona_id?` |
| `PATCH /folders/{id}` | chat | `name`, `parent_id`, `persona_id`, `open` |
| `DELETE /folders/{id}` | chat | delete the folder; its contents move up one level |

Moving is a `PATCH` with a new `folder_id` (a thread follows its parent). The effective persona is computed on read ([db.md](db.md) section 3.1), not copied at creation.

### 5.2 Messages and attachments

| Method and path | scope | Purpose |
|---|---|---|
| `GET /sessions/{id}/messages` | read | history in pages by `id`: `?before=<message_id>&limit=50`, from newest to oldest |
| `POST /sessions/{id}/messages` | chat | send: `body_md`, `client_id` (client UUID, unique within a session), `attachment_ids?`; `202` if the action `send` is in `ask` mode |
| `POST /sessions/{id}/stop` | chat | stop the current agent reply |
| `POST /sessions/{id}/tool-results` | chat | return the result of what the **GUI** executed: agent commands to the browser (`kind = browser`) or user actions on an agent request (`kind = user`); body `ToolResult`; `202` on acceptance and on an identical repeat, `409` if the `request_id` is already closed or the result differs (section 5.2.1). Agent terminal commands are executed by the core, this route does not apply to them |
| `POST /sessions/{id}/attachments` | chat | upload an attachment (`multipart/form-data`, field `file`); response `Attachment` |
| `GET /attachments/{id}` | read | download an attachment |

A repeated `client_id` in the same session with the same text does not create a second message: the core answers `200` and returns the already created one (the first send is `201`); with a different text it answers `409 client_id_conflict`. This makes sending the outgoing queue after a connection loss safe (section 7). The agent reply arrives over WS `session.<id>` (`message.delta` → `message.done`). When `agentd` crashes, messages in status `streaming` are moved to `interrupted` at core start. Attachment limit and allowed types: section 9.

#### 5.2.1 Tool results (`tool-results`)

The agent does not call the API with GUI rights ([ADR-0003](../adr/0003-api-auth.md), token `kind = agent`, section 1.1). Commands to the agent **terminal** are executed by the core in its own pty under uid `agentos-run`; the GUI receives only the `terminal.output` and `terminal.status` frames (section 4.4) and returns no result. Commands to the **browser** are handed to the GUI by the core as a `browser.command` event; the GUI executes them in a separate browser profile and returns the result through this route; the same route returns the result of a user action requested by the agent (for example a file choice).

`ToolResult`: `request_id` (from the event), `kind` (`browser` or `user`), `status` (`ok`, `error`, `timeout`, `cancelled`, `denied`), `output` (page text or HTML, or the value chosen by the user), `truncated`, `output_bytes` (original size), `content_type`, `url` and `http_status` (`browser` only), `error`, `duration_ms`.

- **Size.** `output` up to 256 KiB (working value); the client cuts anything larger and sets `truncated = true`, giving `output_bytes`. A request body above 1 MiB gets `413 payload_too_large`.
- **Owner.** The core accepts a result only from the token that received the event with this `request_id`; otherwise `403 permission_denied`. Scope `chat` is required.
- **Idempotency by `request_id`.** The first acceptance is `202 Accepted` with `{request_id, status: "accepted"}`. A repeat with the same body (after a connection loss, section 7) is again `202`. A different body for the same `request_id` is `409 conflict` with `details.reason = "result_exists"`. A command already finished, expired or cancelled by the core (`stop`, timeout) is `409 conflict` with `details.reason = "request_closed"`. An unknown `request_id` is `404 not_found`.
- **Queue.** The result is put into the outgoing queue with the same `request_id` as the deduplication key; a `4xx` other than `409` is dropped from the queue and the error is shown.

### 5.3 Events

| Method and path | scope | Purpose |
|---|---|---|
| `GET /events` | read | in pages: `?kind&sub&unread&session_id&before&limit`; the response has `items` and `counts` |
| `POST /events/read` | chat | mark as read by filter (`ids`, `kind`, `sub`, `session_id`); an empty body means all unread; response `{ "marked": N }` |
| `POST /events/{id}/read` | chat | mark one event as read |
| `GET /events/subtypes` | read | list of event subtypes as **core data**: `[{ kind, sub, label, icon, enabled }]` (icon and label from the manifest; the GUI does not enumerate providers, [ADR-0001](../adr/0001-init-stack.md)); the notification column sub-filters are built from it |

`counts` are the unread ones: `{ "total": 7, "by_kind": { "ai": 3, "task": 2, "msg": 2 }, "by_sub": { "ai.agent": 3, "task.sched": 1 } }`. An event has a field `tab` (`chat|browser|terminal|settings`, default `chat`): the core computes it, it is not stored in the DB, and the "Open chat" button uses it to lead to the right session and tab.

### 5.4 Permissions and permission requests

| Method and path | scope | Purpose |
|---|---|---|
| `GET /permissions` | read | matrix: `?scope=global|persona|session&scope_id` |
| `PUT /permissions/{scope}/{scope_id}/{action}` | admin, local | set `mode` (`allow|ask|deny`); for `global` `scope_id = 0` |
| `GET /actions` | read | action log: `?session_id&actor&from&to&cursor&limit` (`limit` 1–200); response `{items, next_cursor}`, the cursor is `id` |
| `GET /observer-findings` | read | observer findings: `?kind&verdict&cursor` |
| `GET /memory/facts` · `POST /memory/facts` · `PATCH /memory/facts/{id}` · `DELETE /memory/facts/{id}` | read · chat · chat · chat | memory facts; deleting memory facts is core data, not user files |
| `GET /mcp-servers` · `POST /mcp-servers` · `PATCH /mcp-servers/{id}` | read · admin · admin | registry of MCP servers, local |
| `GET /skills` | read | skills catalog |
| `GET /permission-requests/{id}` | read | current state of a request (to sync clients under first-wins) |
| `POST /permission-requests/{id}/decision` | admin, local | user decision; body `{ decision: allow|deny|always, rule? }`, with `always` `rule = { scope, scope_id, mode? }` (`mode` defaults to `allow`) |

Rules ([ADR-0005](../adr/0005-permissions-semantics.md)):

- The effective mode is the strictest of `global → persona → session` (`deny` > `ask` > `allow`). **A lower level can only tighten:** an attempt to loosen (including a `PUT` of a session row with `allow` while a stricter level exists above) is rejected with `403 permission_denied`, and the core writes a `permission_loosen` finding to `observer_findings` (the name from ADR-0005, the same in [db.md](db.md) section 3.11).
- **"Always"** (`decision = always`) does not loosen the session level. It creates or changes a persona or global rule: body `rule = { scope: "persona" | "global", scope_id, mode }` (`mode` defaults to `allow`; for `global` `scope_id = 0`); the user chooses the level explicitly in the card; `scope: "session"` is not allowed (`422`). The core applies it as `PUT /permissions/{scope}/{scope_id}/{action}` with `allow`; if that would loosen a global `deny`, the answer is `403`.
- **The first decision wins:** two local clients with an `admin` token (two GUI windows, the GUI and a local CLI) may answer the same request at the same time; the second gets `409 conflict` (`details.reason = "already_decided"`), and `permission.resolved` arrives over WS. **The phone and other external clients do not decide requests:** an external token is no higher than `chat`, while `decision` requires `admin` and the local socket; a separate scope `decide` is outside 0.1.
- An unanswered request expires at `expires_at` (**10 minutes**, [ADR-0005](../adr/0005-permissions-semantics.md)) and counts as `deny`, status `expired`.
- When the observer is unavailable, dangerous actions (`delete`, `pay`, `shell`, sending outward) are **fail-closed:** the action is not performed, and the request is returned as `ask` with a note in `detail`.
- Decisions and permission changes are available only to an `admin` token on the local socket: the agent shell (separate uid, token `kind = agent`) and external clients cannot make them.

**Request storage.** `PermissionRequest.id` is `actions_log.id` (the record is created when the request arises; until decided the `decision` field is empty; after the decision it becomes `ask-approved` / `ask-denied` / `deny`). The chat card is a message with `role = system`, `meta.kind = "permission_request"` and `meta.request_id`. For requests without a session (module install, cron actions) there is no card; the request arrives in an event and in the `202` response. The request state is stored in the table `permission_requests` ([db.md](db.md) section 3.14): `status` (`pending|allowed|denied|expired|cancelled`), `decided_by`, `decided_at`, `expires_at`; `actions_log` receives the outcome after the decision. A request contains `options[]` (`allow_once|allow_always|reject_once|reject_always`, modeled on ACP); when a turn is stopped (`stop`) open requests become `cancelled` and WS `permission.resolved` is sent.

Fields for observer cards: `source = observer`, `kind` (`injection|phishing|dangerous_cmd|module|permission_loosen`), `verdict` (`blocked|asked|allowed`) from `observer_findings`; `status` (`pending|allowed|denied|expired`), `decided_by`, `decided_at`.

### 5.5 Search

`GET /search?q=<text>&limit=20` (read) is the unified Ctrl+K search over sessions, messages, mail, files and memory (core RAG). Response: `{ "items": [{ "kind": "session|message|mail|file|memory", "source": "local|mcp_memory", "title", "snippet", "ref": { … } }] }`. `source` shows where a memory record came from: the local index (`index.db`) or the MCP server `memory_mcp` ([ADR-0009](../adr/0009-memory-via-mcp.md)); the choice is set by the setting `memory.source` (`local`, `mcp_memory`, `both`), settings page `memory` (section 5.7). For every `kind` other than `memory`, `source = local`.

### 5.6 Personas, accounts, quotas, modules

| Method and path | scope | Purpose |
|---|---|---|
| `GET /personas` · `POST /personas` | read · admin | list · create |
| `PATCH /personas/{id}` · `DELETE /personas/{id}` | admin | change (including `account_ids`) · delete |
| `GET /accounts` | read | accounts without secrets (`secret_ref` is not returned) |
| `POST /accounts` | admin | create an account (`type`, `label`) in status `disconnected`; the secret goes through `PUT /secrets/{ref}` |
| `PATCH /accounts/{id}` · `DELETE /accounts/{id}` | admin | label · delete (the `gw_messages` archive is not deleted) |
| `POST /accounts/{id}/connect` | admin | start connecting (OAuth or QR); `202` with `ConnectFlow`, progress over WS `connections` |
| `DELETE /accounts/{id}/connect` | admin | cancel the current flow |
| `GET /quotas` | read | same as the `bar.quota` snapshot (accounting by fact, no subscription remainder); `?session_id=<id>` gives `usage` for that session (`period = session`) instead of the day |
| `GET /modules` | read | installed ones and the catalog |
| `POST /modules` | admin | install a custom module by URL: `url`; the core hands the module to the observer for checking, answer `202` (`PermissionRequest`) or `201` |
| `PATCH /modules/{module_id}` | admin | `enabled`, `placement`, `config`; `module_id` is a string |
| `DELETE /modules/{module_id}` | admin | uninstall (the module returns to the catalog) |

### 5.7 Settings, secrets, files, tunnels

**Settings pages.** Conditional requests (section 3): `GET` returns `ETag`, `PATCH` (JSON merge patch) requires `If-Match`.

| Method and path | scope | Purpose |
|---|---|---|
| `GET /settings/{page}` | read | the whole page |
| `PATCH /settings/{page}` | admin; a web token `decide` only for pages `account`, `personas`, `schedule`, `memory`, `indicators` | change the page; the response is the saved document with the new `rev`; `202` on `config_write` in `ask` |
| `GET /settings/indicators` · `PATCH /settings/indicators` | read · admin | bar indicators: `{ rev, items: [{id, enabled, position, config?}] }` |
| `GET /settings/theme` · `PATCH /settings/theme` | read · chat | `{ rev, theme: "auto|dark|light" }`; the source of truth is the core (`settings.theme`), the GUI caches the value in `ui.json` and applies it at start without a connection |

Pages `{page}`: `account`, `about` (read-only), `personas` (including the global balance), `models`, `connections`, `permissions`, `security`, `schedule`, `modules`, `tunnels`, `backup` (the field `last_restore_test_at` is an integer, unix seconds of the last restore check, or `null`; older than 35 days turns the `backup` indicator red), `memory` (`memory.source`: `local`, `mcp_memory`, `both`; address and status of the MCP server `memory_mcp`, [ADR-0009](../adr/0009-memory-via-mcp.md)). `indicators` and `theme` are separate routes with their own schemas (they used to duplicate the template path and conflict with it). The "Files" page does not use `/settings/files` and works through `/files` below. Save errors are `422` with `details` per field, and the client shows them in the dialog.

**Secrets** ([ADR-0003](../adr/0003-api-auth.md)):

| Method and path | scope | Purpose |
|---|---|---|
| `GET /secrets` | admin, local | which secrets are set: `[{ ref, set, updated_at }]`, values are not returned |
| `PUT /secrets/{ref}` | admin, local | write a value `{ value }`; **the value is never read back** |
| `DELETE /secrets/{ref}` | admin, local | delete |

`ref` is a reference such as `provider.anthropic`, `account.3`; tables store only `secret_ref`.

**Files.** The "Files" editor opens only `AGENT.md` and `memory/`; any other path is `403 permission_denied`. `keys.env` and `~/.agentos/*.json` are not available through the editor.

| Method and path | scope | Purpose |
|---|---|---|
| `GET /files` | admin, local | list of editable files `[{ path, kind, size, rev }]` |
| `GET /files/content?path=` | admin, local | content, `ETag` header |
| `PUT /files/content?path=` | admin, local | write the whole file; `If-Match` is mandatory |

**Tunnels.**

| Method and path | scope | Purpose |
|---|---|---|
| `GET /tunnels` · `POST /tunnels` | admin | list · add: `type` (`wireguard|tailscale|ssh|cloudflare|ngrok`), `addr`, `descr?`, `enabled?` |
| `PATCH /tunnels/{id}` · `DELETE /tunnels/{id}` | admin | enable, disable, change · delete |

Enabling access to the core API from outside (`tunnels.api_external`) is a field of the settings page `tunnels`.

### 5.8 Tasks, bookmarks, tokens

| Method and path | scope | Purpose |
|---|---|---|
| `GET /sessions/{id}/tasks` · `POST /sessions/{id}/tasks` | read · chat | session tasks · create (`kind`, `title`, `spec`, `cron?`) |
| `GET /tasks` | read | all tasks (`?kind&state&session_id`), the "Schedule" page |
| `PATCH /tasks/{id}` · `DELETE /tasks/{id}` | chat | change, pause and resume (`state`) · delete with the run history |
| `GET /tasks/{id}/runs` | read | run history (`?before&limit`) |
| `POST /tasks/{id}/run` | chat | run now; `202` with `TaskRun` |
| `GET /triggers` · `POST /triggers` | read · admin | event triggers · create |
| `PATCH /triggers/{id}` · `DELETE /triggers/{id}` | admin | change, enable and disable · delete |
| `GET /sessions/{id}/bookmarks` · `PUT /sessions/{id}/bookmarks` | read · chat | browser bookmarks of the session `[{title, url}]`; `PUT` with `If-Match` |
| `GET /browser/quicklinks` · `PUT /browser/quicklinks` | read · chat | quick links of the start page; `PUT` with `If-Match` |
| `GET /tokens` · `POST /tokens` | admin, local | list without values (all `kind`, including `agent` and `module`) · issue an external client token (`name`, `scope` ≤ `chat`, `ttl_days` ≤ 90, `kind` `external`; with `decide: true` it is `kind = web`, `scope = decide`, `ttl_days` 1–30, default 7) |
| `DELETE /tokens/{id}` | admin, local | revoke |

### 5.9 Deletion

- Deleting a session deletes its threads and messages; the events of this session **remain** with `session_id = null`; action log records remain too ([db.md](db.md) sections 5, 3.3, 3.7). The session bookmarks and the browser profile of this session are deleted with it. Open permission requests of this session are resolved as `deny`.
- Before `DELETE` the client shows a confirmation listing what will be lost.

### 5.10 Service

`GET /health` is described in section 2 (anonymous). `POST /logout` (admin, local) ends the user session (menu Agent OS → Logout).

`GET /v1/metrics` (admin, local) returns counters and histograms of the core without user data (number of requests, errors by `code`, latencies, queue sizes, `user_version`); text exposition format or JSON, defined by the implementation. The same set goes into the `--diagnose` report. Request values, session names and message texts are not present in the metrics.

## 6. Object schemas

Consistent with the tables of [db.md](db.md); fields not from the DB are marked "computed".

| Object | Fields |
|---|---|
| **Folder** | `id`, `parent_id`, `name`, `persona_id`, `open`, `created_at` |
| **Session** | `id`, `folder_id`, `parent_id` (not `null` = thread), `name`, `persona_id`, `effective_persona_id` (computed), `path`, `cfg` (`model` is an `account_id`, `perms`, `mem`, `notif`, `ctx`, `auto`), `unread`, `archived`, `created_at`, `updated_at` |
| **Message** | `id`, `session_id`, `client_id` (`null` for agent messages), `role` (`user|agent|system|tool`), `author`, `body_md`, `status` (`streaming|done|stopped|error|interrupted`), `meta` (permission cards, observer confirmations, attachments), `ts` |
| **Attachment** | `id`, `name`, `mime`, `size` |
| **Event** | `id`, `kind` (`ai|task|msg`), `sub`, `level` (`info|warn|error|ok`), `text`, `session_id`, `tab` (computed: `chat|browser|terminal|settings`, default `chat`), `source`, `unread`, `ts` |
| **Persona** | `id`, `name`, `color`, `model_acc`, `style`, `lang`, `signature`, `mem_mode` (`own|shared|none`), `notif_mode` (`all|important|digest|off`), `work_hours`, `quiet_hours`, `account_ids` |
| **Account** | `id` (integer), `type` (`wa|tg|gmail|anthropic|ollama|…`), `label`, `status` (`connected|disconnected|qr|error`), `created_at` |
| **ConnectFlow** | `account_id`, `kind` (`oauth|qr|token`), `status` (`pending|waiting_user|connected|failed|expired`), `url`, `qr_data`, `expires_at` |
| **Quota** | `account_id`, `kind` (`api_key` or `local`), `window_h` (0 = local, the window of the nearest limit in hours, `null` if the limit is unknown), `used` (0..1 by the tightest limit, `null` if the limit is unknown), `rate` (× of the allowed one, `null` if the limit is unknown), `reset_at`, `eta_at` (computed, `null` if it lasts until reset), `load` (local only, 0..1), `usage` (`QuotaUsage`), `limits` (`QuotaLimits`), `updated_at` |
| **QuotaUsage** | `period` (`day` or `session`), `session_id` (with `session`), `tokens_in`, `tokens_out`, `cost`, `currency` |
| **QuotaLimits** | `requests_per_min`, `tokens_per_min`: each `{limit, remaining, reset_at}` or `null` (from the rate-limit headers of provider responses) |
| **ToolResult** | `request_id` (string), `kind` (`browser|user`), `status`, `output`, `truncated`, `output_bytes`, `content_type`, `url`, `http_status`, `error`, `duration_ms` (section 5.2.1) |
| **Health** | `status` (`ok`, `starting`, `degraded`, `migrating`), `version`, `api {major, minor}`, `min_client`, `min_gui`, `max_gui`, `user_version`, `capabilities[]`, `migration` (`null` or `{status, from, to, step?, progress?}`) (section 2.1) |
| **Permission** | `scope` (`global|persona|session`), `scope_id`, `action` (`read|send|delete|pay|shell|browse|memory_write|config_write`), `mode` (`allow|ask|deny`) |
| **PermissionRequest** | `id` (= `actions_log.id`), `session_id`, `action`, `target`, `source` (`policy|observer`), `level` (`ask|warn`), `kind`, `verdict`, `status` (`pending|allowed|denied|expired`), `detail`, `decided_by`, `decided_at`, `expires_at`, `created_at` |
| **PermissionDecision** | `decision` (`allow|deny|always`), `rule` (`scope` = `persona|global`, `scope_id`, `mode` = `allow|ask|deny`, default `allow`; mandatory with `always`) |
| **Module** | `id` (string), `name`, `manifest`, `installed`, `enabled`, `placement` (`panel|widget|button`), `config` |
| **Indicator** | `id` (string: `cpu`, `whapi`, `quota.cl1`), `enabled`, `position`, `config` (thresholds, period, `click_action`) |
| **IndicatorState** | `id`, `state` (`ok|warn|bad|nodata|unknown`), `value`, `text`, `ts` |
| **Task** | `id`, `kind` (`sched|deleg`), `title`, `spec`, `cron`, `tz`, `state` (`active|paused|done`), `persona_id`, `session_id`, `next_run`, `created_at` |
| **TaskRun** | `id`, `task_id`, `started_at`, `finished_at`, `status` (`running`, `ok`, `error`, `skipped`), `output` |
| **Trigger** | `id`, `source` (`mail|wa|tg|git|…`), `match` (object), `session_id`, `enabled` |
| **Tunnel** | `id`, `type` (`wireguard|tailscale|ssh|cloudflare|ngrok`), `addr`, `descr`, `enabled`, `status` (computed) |
| **Token** | `id`, `name`, `kind` (`gui|external|test|agent|module`), `scope` (`read|chat|admin`), `allowlist` (array of strings `METHOD path`, only `agent` and `module`, otherwise `null`), `created_by`, `created_at`, `expires_at`, `last_used`, `revoked`; on issue also `token` (once; the value is never shown again) |
| **SecretInfo** | `ref`, `set`, `updated_at` |
| **FileInfo** | `path`, `kind` (`prompt|memory`), `size`, `rev` |
| **Link** | `title`, `url` |
| **EventCounts** | `total`, `by_kind`, `by_sub` |

Mapping of `Quota` to the columns of `quotas` in [db.md](db.md): `kind` ← `kind`, `load` ← `load`, `usage.tokens_in`/`tokens_out`/`cost` ← `usage_tokens`, `usage_cost`, `limits.requests_per_min` ← `limit_requests`, `limits.tokens_per_min` ← `limit_tokens`, `limits.*.remaining` ← `remaining`, `limits.*.reset_at` and `reset_at` ← `reset_at`; `window_h`, `used`, `rate` allow NULL.

Not returned to the client: `secret_ref`, values from `keys.env`, secret values, `dedup_key`, token hashes, the content of the log `args_json` without masking ([db.md](db.md) sections 1, 3.3, 3.7).

## 7. Connection loss

Client behavior ([architecture.md](../architecture.md)):

1. **Detection:** WS closed, socket write error, two consecutive missed `pong` (the `ping` interval is 15 s, a working value).
2. **State:** `ConnectionController` publishes `connection.offline`; indicators turn gray ("no data"), the banner "No connection" is shown. The GUI does not crash and loses nothing.
3. **Outgoing:** what the user typed goes into a queue with `client_id`; a `POST` with a network error or `5xx` stays in the queue, a `4xx` does not (an error is shown in the chat). The queue is not replayed blindly: if the session was deleted or permissions changed, the repeat gets `404`/`403` and the user sees an error; decisions on permission requests are not queued (they are online only).
4. **Reconnect:** retries with growing pauses (working series: 1, 2, 4, 8, 16, 30 s, ceiling 30 s) and random jitter, **with no limit on the number of attempts** (the `reconnecting` state does not become a final `offline` by a counter).
5. **Recovery:** `connection.online`, `subscribe` again to all active topics, receiving a `snapshot` for each, then sending the queue in the original order and removing the banner.
6. **WS close codes (working):** `1000` normal, `1001` the core is restarting (reconnect), `4401` token rejected or revoked (do not retry, show an access error), `4403` insufficient scope or the `Origin`/`Host` check failed (do not retry), `4426` API version not supported (do not retry).
7. **The core is starting:** `503 core_down` until ready; the client retries as when there is no connection. The connection states in the GUI are one set: `connecting`, `connected`, `reconnecting`, `offline`, `core_unavailable` ([ADR-0006](../adr/0006-kiosk-safety.md): the core does not start, the "Diagnostics" button and the recovery instructions); the HTTP code of this state is `core_down`.
8. **The core is migrating:** `503 migrating` (or `/health` with `status = "migrating"`) is not a connection loss: the "No connection" banner is not shown, the GUI shows "The core is updating the database" with the progress `migration.progress`, the outgoing queue is not dropped and goes out after `status = "ok"` (section 2.1).

## 8. Contract security

- A token is always required ([ADR-0003](../adr/0003-api-auth.md)): the `0600` socket and `SO_PEERCRED` are additional protection, not a replacement for the token. Any TCP access is only with a token, only through a tunnel, with `Origin` and `Host` checks.
- The API passes user decisions to the core but not secrets; neither the GUI nor the agent can widen a `deny` or loosen a rule at a lower level (section 5.4).
- Only `admin` on the local socket can decide permission requests and change permissions; the agent shell (separate uid, token `kind = agent`) and external clients (phone) cannot.
- All text fields (names, `body_md`, event `text`) are untrusted: the client escapes them on output ([architecture.md](../architecture.md)).
- Dangerous agent actions (sending, deleting, payments, config, terminal commands) go through permissions and the observer; the API only passes the request and the decision. When the observer is unavailable, dangerous actions are not performed (fail-closed).
- Conditional requests (`If-Match`) protect settings from lost updates between the GUI and the phone.

## 9. Divergences and open questions

**Settled in this revision:**

- `Event.tab` is a computed core field, `chat` by default, not stored in the DB; `target.tab` from the earlier sprint draft is replaced by `Event.tab`.
- Tokens, scope, `Origin`/`Host`; only `GET /health` is anonymous ([ADR-0003](../adr/0003-api-auth.md)). Token format: an opaque string, the core stores the hash; lifetime 30 days by default.
- Permissions: "Always" is a persona or global rule, a lower level only tightens, the first decision wins (`409`), the request is stored in `actions_log` ([ADR-0005](../adr/0005-permissions-semantics.md)).
- Message idempotency and streaming: `client_id`, `offset`, `final_len`, status `interrupted`.
- `counts` (not `count`), `bar.quota`, `bar.weather`, the full topic list, `PATCH` + `If-Match` for settings, routes for tasks, triggers, bookmarks, attachments, account connection, tunnels, secrets, files and tokens.
- Q6 ([ADR-0008](../adr/0008-open-questions-closed.md)): `POST /sessions/{id}/tool-results`, extended `GET /health` (`api {major, minor}`, `min_client`, `min_gui`/`max_gui`, `capabilities`, `migration`), code `migrating`, header `X-Request-Id`, `usage` and limits in `Quota` without a subscription remainder (Q1).
- Conflict reconciliation: the GUI token is not in a file but in memory (fd or socketpair); agent terminal commands are executed by the core, the GUI receives `terminal.output` and `terminal.status`, and `tool-results` exist only for `browser` and `user`; `kind = agent|module` and `allowlist` on tokens; decisions on permission requests only by `admin` locally, the phone does not decide; 412 and 428 for `If-Match`, request TTL 10 minutes, theme in the core, indicator states `ok|warn|bad|nodata|unknown`; code `core_down` and one set of connection states; routes `GET /events/subtypes`, `GET /v1/metrics`, `SearchHit.source`, settings page `memory`, `?token=` → `401`.

**Open:**

1. **Columns in [db.md](db.md)** that the core needs for this contract and that db.md must have: `quotas` (`kind`, `load`, `usage_tokens`, `usage_cost`, `limit_requests`, `limit_tokens`, `remaining`, `reset_at`, NULL for `window_h`, `used`, `rate`), `api_token` (`kind` includes `agent` and `module`, field `allowlist_json`), `settings.rev` and the `memory` pages. These are fixed in db.md, not here.
2. The 256 KiB output size and the 1 MiB body limit are working values; the agent terminal as a separate core pty under uid `agentos-run` and the `agent-runner` link are confirmed by the implementation ([ADR-0003](../adr/0003-api-auth.md)).
3. **Attachment limits:** size (working value 25 MiB), allowed types, storage location.
4. **Pagination:** the `before` cursor is accepted by `id`; for search and events the exact cursor scheme is not fixed.
5. **Idempotency of other `POST`s** (creating a session or folder): `client_id` is defined only for messages.
6. **The format of Settings page documents** is defined by the `agentd` implementation; in the contract they are an `object` with no strict schema (except `indicators` and `theme`).
7. Values of timeouts, pauses, attempt limits (5 per minute over TCP) and WS close codes (section 7, section 1.1) are working ones, to be confirmed against the implementation; the 10-minute TTL of a permission request is accepted ([ADR-0005](../adr/0005-permissions-semantics.md)).
8. **Weather source** (`bar.weather`): core or module, not defined.
9. The `agt_` + base64url token format and the 30/90-day lifetimes are working values from [ADR-0003](../adr/0003-api-auth.md).

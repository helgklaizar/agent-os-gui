# ADR-0003. Core API authentication

Status: accepted

## Context

The GUI talks to `agentd` over the unix socket `$XDG_RUNTIME_DIR/agentd.sock` ([ADR-0001](0001-init-stack.md)). From outside, the API is reachable over TCP through a tunnel. The first draft of the API ran the local socket without a token, and the OpenAPI draft had an empty root `security`, i.e. anonymous access to all paths. A security audit found two critical consequences:

1. A `0600` socket without a token is open to any process of the same user. The agent runs shell commands under the same uid, so it can call `POST /permission-requests/{id}/decision`, `PUT /permissions` and `POST /logout` itself. The `ask` model and the observer are bypassed with one `curl --unix-socket`.
2. Anonymous TCP through Cloudflare or ngrok exposes machine control. There is no token, no lifetime, no `Origin`/`Host` check, and a page inside WebKit can reach `127.0.0.1:4096`.

Spikes on Ubuntu 24.04 refined this:

- A process of the same uid **can read** both a `0600` token file and `/proc/PID/environ` of the GUI process. `SO_PEERCRED` returns the same uid for it, so the agent cannot be told from the GUI by uid. A process under a different uid (`agentos-run`) can read neither.
- Unprivileged `bwrap` does not work on 24.04 (`apparmor_restrict_unprivileged_userns=1`: `setting up uid map: Permission denied`). A working isolation for agent commands is a system transient unit: `systemd-run --uid=agentos-run` with `ProtectHome`, `ProtectSystem=strict`, `ReadWritePaths`, `NoNewPrivileges`, `IPAddressDeny`.

## Decision

### Tokens and scopes

A token is always required, on the unix socket and over TCP, sent as `Authorization: Bearer <token>`. Scopes, ordered `read < chat < decide < admin`:

| Scope | Allowed |
|---|---|
| `read` | reads: sessions, messages, events, settings (without secrets), WS topic subscription, `GET /health` |
| `chat` | `read` + send messages, stop a stream, create and edit sessions, folders, threads, `PATCH` session settings |
| `decide` | `chat` + a one-time decision on a permission request (`POST /permission-requests/{id}/decision` with `allow` or `deny`); "Always" and other `admin` routes are unavailable |
| `admin` | `decide` + `permissions`, `logout`, `files`, `secrets`, `tunnels`, `modules`, issuing and revoking tokens |

`permissions`, "Always" decisions, `logout`, `files` and `secrets` answer only to `admin`. Anonymous: `GET /health` (no user data) and, when `--web-root` is set, static `GET /` and `GET /app/*` (client code, not data).

### How the GUI gets its token

- **The GUI token is not stored in a file.** A `0600` file in `$XDG_RUNTIME_DIR` and environment variables are readable by any process of the same uid, including the user's terminal and the GUI's child processes. So `agentd` hands the token to the GUI at launch through an inherited file descriptor (anonymous `memfd`/`pipe` or one end of a `socketpair`). The value is read once into GUI memory and kept only there. It never appears in `environ`, `cmdline`, a `/proc/PID/fd` file target, or `$XDG_RUNTIME_DIR`.
- **Launch under cage.** The GUI process is a descendant of the core: `greetd` starts `agentd gui-launch` in cage; it opens a `socketpair` to the running `agentd`, receives the descriptor with the token and `exec`s `agent-shell` with the inherited fd. Fallback if that proves infeasible: a one-time launch code (valid 10 s, one exchange), where trust then rests on same-uid processes being trusted. A prototype confirmed the fd scheme.
- The GUI token has scope `admin`, `kind = gui`, lives until the GUI process stops; on unit restart a new one is issued and the old one revoked.
- Protection against agent processes rests not on hiding the token but on agent commands running under a different uid (below). Everything running under the user's uid that is not the GUI (the user's terminal and programs) is trusted as much as the user.
- Tokens are never written to disk in `~/.agentos/`; the DB stores only a SHA-256 hash and metadata (scope, expiry, `last_used`, `revoked`) in the `api_token` table ([db.md](../reference/db.md)).
- **Tokens in the DB are SHA-256 only (hex).** The raw value (prefix `agt_` and 32 random bytes from the OS generator) is shown **once**, in the issue response (`POST /tokens` or the GUI token handover), and cannot be recovered: a lost token is reissued. Hashes are compared in constant time. `actions_log` and logs record only the token id and label, never the value or hash.
- **No tokens in URLs.** A request with `?token=` or `?access_token=` (including percent-encoded) is rejected with `401` without touching the table, because URLs end up in proxy logs. Only the `Authorization: Bearer` header is accepted.
- Extra check on the socket: `SO_PEERCRED` compares the client uid with the owner of `agentd`; a token without a matching uid is rejected. This guards against other users, not against same-uid processes, so it does not protect against the agent without a separate uid.

### Token kinds

`api_token.kind`:

| `kind` | For | Scope | Notes |
|---|---|---|---|
| `gui` | the `agent-shell` process | `admin` | handed over via descriptor at launch, no expiry, lives until the process stops |
| `external` | phone, CLI, external client | at most `chat` (`CHECK external <> admin`) | 30 days by default, revocable, attempt limit over TCP |
| `agent` | the agent's command-execution process | `chat`, restricted by `allowlist_json` | issued by the core over a separate channel when the command unit starts, lives until the unit ends, cannot decide permission requests |
| `web` | browser client ([ADR-0010](0010-web-client.md)) | `chat` or `decide` | expiry mandatory: 7 days by default, 30 maximum; issued by `POST /tokens` with `decide: true` or `agentd token issue --decide` (local, `admin` only); never `admin` (`CHECK`) |
| `module` | module panel ([ADR-0004](0004-modules-sandbox.md)) | `chat` or lower, restricted by `allowlist_json` | issued when the panel opens; modules never get `admin` |
| `test` | `fake-agentd` and contract tests | any | test build only |

`allowlist_json` is a list of permitted bridge methods or routes; a request outside it gets `403 forbidden_method` and is logged.

### Isolation of agent commands

- **The core executes, not the GUI.** Agent commands (terminal, shell, tools) are launched by `agentd` in its own pty under a separate uid `agentos-run`. The GUI only displays output (`terminal.output` frames) and has no launch right. The user's own terminal (VTE) and browser results return to the core via `tool-results`, but that is not an agent command channel.
- **Mechanism.** A system transient unit `systemd-run --uid=agentos-run` with `ProtectHome=yes`, `PrivateTmp=yes`, `ProtectSystem=strict`, `ReadWritePaths=<session project dir>`, `InaccessiblePaths=/run/user`, `NoNewPrivileges=yes`, and `IPAddressDeny` where needed. Verified: writing only into the project directory and `git commit` work; `~/.ssh`, tokens and `/run/user` are inaccessible; the network can be closed. The project directory must be outside `/tmp` and `/home` (or attached with `BindPaths`).
- **`bwrap` only with an AppArmor profile.** Unprivileged `bwrap` is blocked on Ubuntu 24.04+; it is allowed only with the AppArmor profile shipped in the package.
- **Privileged broker `agent-runner`.** `agentd` runs as a user unit and cannot call the system `systemd-run` itself. A small system unit `agent-runner` with a socket accepts a request from `agentd`, checks `SO_PEERCRED` (the uid of the `agentd` owner) and starts the transient unit under `agentos-run`. A prototype confirmed it as a root service with socket activation, an allowlist of directories owned by the requester, and `systemd-run --uid=agentos-run` with `ProtectSystem=strict`, `ProtectHome=tmpfs`, `BindPaths`, `IPAddressDeny=any`, `NoNewPrivileges`. It works for projects under `/home` (ACL on the project directory). Open: ownership of created files, allowlist replenishment, one uid for all projects. See also [agentd-A-0006-runner-packaging.md](agentd-A-0006-runner-packaging.md).
- **Mandatory runner hardening.** (1) Guard against swapping the project directory after the allowlist check: open it with `O_PATH|O_NOFOLLOW` and `mount --no-canonicalize --bind /proc/PID/fd/N`. (2) Peer check is not a denylist: the `agentd` group on the socket (`0660`) plus directory ownership. (3) `RuntimeMaxSec` and unit stop. (4) Validate `allow.list`. (5) Request size limit, no tracebacks to the client. (6) Default ACL on the project.
- **What the agent cannot see.** The user's `$XDG_RUNTIME_DIR`, the GUI token descriptor, `~/.agentos/keys.env`, `~/.ssh`, the `agentd` socket with an `admin` token. If the agent needs the core API, it gets a `kind = agent` token with scope `chat` over a separate channel and cannot decide or change permissions. Access to the user's ssh and git goes through a deploy key or ssh-agent; handing out the user's rights is forbidden.
- The fd scheme holds under `kernel.yama.ptrace_scope=1` and an ancestor check of the requesting process (otherwise the first same-uid process to ask gets the token).

### TCP through a tunnel

- TCP listens only when enabled in Settings, Tunnels; it is off by default.
- Every connection needs a scoped token; `Origin` and `Host` are checked against an allowlist (tunnel addresses) and mismatches get `403`; pages from WebKit (`file://`, custom schemes, `localhost`) are rejected without explicit permission.
- `logout`, `files`, `secrets`, `permissions` and token issuing are unavailable over TCP; they need the local socket.
- External-client tokens are issued by `POST /tokens` (local, `admin`) with scope at most `chat`.
- **Exception for `web` tokens with scope `decide`:** a one-time permission decision is accepted over TCP too (other local-only routes stay closed); the decision is logged in `actions_log` as `web:<token id>`; `always` from such a token gets `403`.
- **Browser sign-in:** the token is typed into a form and kept in tab memory (optionally `sessionStorage`); it never enters the URL. For WS the token travels in `Sec-WebSocket-Protocol: agentd.v1, bearer.<token>` (browsers cannot set `Authorization`); the core answers with subprotocol `agentd.v1` and never echoes the token.

### Lifetime and revocation

- External client tokens live 30 days by default, 90 at most. Renewal is issuing a new token and revoking the old one (no renewal route).
- Revocation: `DELETE /tokens/{id}` (`admin`, local). A revoked token is rejected immediately and its WS connections close with code `4401`.
- Failed-attempt limit over TCP: 5 per minute per address, then a pause; on excess, `429` with `Retry-After` in seconds (at least 1) and `request_id` in the error body. The client address is taken from `X-Forwarded-For` right to left, skipping trusted proxies (by default only loopback and the tunnel address from Settings), otherwise all clients share one bucket. Tokens that are not in the table are counted in the address bucket and are not "remembered" by the limiter.

### Secrets

Secret values (API keys, messenger tokens) are written only through `PUT /secrets/{ref}` and never read back: the response has `ref`, `set: true` and the modification time. `accounts` stores `secret_ref`, not the value.

## Trust boundary: `agentd` itself

The model above protects the GUI and core **from agent commands** (different uid), not from `agentd` itself: it reads mail, messages and pages, i.e. receives untrusted text (prompt injection). Decided:

- `agentd` is trusted by code, but its **input is untrusted**: any text from external sources is marked `untrusted` and cannot grant permissions, change persona rules or request tokens by itself.
- An action with an outward effect (sending, payment, writing outside the project, `secrets`) goes only through a permission request ([ADR-0005](0005-permissions-semantics.md)) with the request source shown in the card.
- Compromise of the `agentd` process itself (RCE in the core) is outside the 0.1 model. Mitigation: a minimal dependency set, `deny.toml`, a user unit with `NoNewPrivileges` and `ProtectSystem=strict`; the residual risk is accepted. See [security-model.md](../reference/security-model.md).

## Consequences

- A user process without a token can no longer decide permission requests, provided agent commands are really isolated by uid.
- The OpenAPI document has no `{}` in the root `security`; `/health` has `security: []`; [api.md](../reference/api.md) describes scopes and codes `401`, `403`, `4401`, and `?token=` is rejected.
- `fake-agentd` needs the same token mechanism, otherwise contract tests do not check authorization.
- Uid isolation becomes a requirement for `agentd` and packaging: an `agentos-run` user, the `agent-runner` unit and an AppArmor profile in the package. Without it the security model holds only on paper.
- The GUI token exists only in GUI process memory: if the GUI crashes, the unit restarts and gets a new token; the old one is revoked.
- Token lifecycle tests (hash in DB, `?token=` gives `401`, `4401` on revocation, `429` with `Retry-After`, `SO_PEERCRED`, expiry, `CHECK external <> admin`) are part of the first milestone; see [testing.md](../testing.md).

## Alternatives

- **Socket without a token, protected by file permissions.** Rejected: does not protect against an agent under the same uid.
- **One token for everything.** Rejected: leaking an external client token would hand out `admin` actions.
- **`SO_PEERCRED` only, no token.** Rejected: cannot tell GUI from agent under one uid and does not work for TCP.
- **mTLS for TCP.** Deferred: tunnels (Cloudflare, ngrok) terminate TLS themselves; it can be added on top of tokens later.

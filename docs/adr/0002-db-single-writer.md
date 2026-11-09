# ADR-0002. `agentd` is the only DB writer

Status: accepted

## Context

The design describes several user daemons under `systemd --user` (`agentd`, `agent-watch`, `agent-gw`, `agent-sched`, `agent-index`, `agent-tunnel`, `agent-sys`, `agent-backup`) and a single `sessions.db` (SQLite: sessions, threads, history, events). The daemons overlap by subsystem. If each opens `sessions.db` for writing, there are lock races, schema drift when daemons are upgraded separately, and no way to guarantee permission rules and the action journal (every agent action logged with undo). See [db.md](../reference/db.md).

## Decision

1. `sessions.db` (SQLite, WAL; the file and `-wal`/`-shm` are `0600`, the directory `~/.agentos` is `0700`) is written **only by `agentd`**. Other daemons and the GUI change data through the core's HTTP/WS API: the unix socket `$XDG_RUNTIME_DIR/agentd.sock` locally, TCP `127.0.0.1:4096` through a tunnel from outside. Tokens are mandatory ([ADR-0003](0003-api-auth.md)).
2. Inside `agentd`, writes go through one queue and one write connection; reads use a connection pool.
3. Only `agentd` applies schema migrations, at startup. Files are named `NNNN_name.sql` (four digits), the version is recorded in `schema_migrations`, and `PRAGMA user_version` is set only by the runner after a successful migration. A DB newer than the code refuses to start.
4. Exceptions: `agent-backup` reads a snapshot via `VACUUM INTO` (no writer lock); `agent-index` owns a **separate** DB `index.db` (SQLite + `sqlite-vec`) and never writes to `sessions.db`.
5. The single write point enforces: permission checks (a lower level may only tighten; an attempt to loosen gives `403` and a row in `observer_findings`, see [ADR-0005](0005-permissions-semantics.md)), `actions_log` entries with `undo_json`, message idempotency (`client_id`), "one transition, one event" (`indicators.last_state`, `dedup_key = <indicator>:<transition_seq>`), and tree integrity (triggers against cycles and nested threads).
6. Secrets are not stored in the DB: `accounts.secret_ref` points to `keys.env` (`0600`, unreadable by the agent uid) or a keyring; secrets are written through the write-only `PUT /secrets/{ref}`.
7. Deletion never loses data silently: accounts and personas are soft-deleted (`deleted_at`); `gw_messages` and `memory_facts` are protected from cascades with `RESTRICT`; tasks and triggers of a deleted session are paused, not erased.
8. After migrations at startup, `agentd` moves `streaming` messages to `interrupted` (the core crashed mid-reply).

## Consequences

- Pros: no write conflicts; the schema changes in one place; the journal and permissions cannot be bypassed by direct writes; GUI and phone see the same state; daemons do not depend on the DB format and can be upgraded independently while the API is stable.
- Cons: `agentd` is a bottleneck and single point of failure; daemons need a working API (if `agentd` is down, daemons must buffer events and messages themselves). API authentication is required (solved in [ADR-0003](0003-api-auth.md)), and several users on one machine each get their own `~/.agentos/` and socket.
- Database encryption at rest is settled in [ADR-0008](0008-open-questions-closed.md): SQLCipher is not used in 0.1; full-disk encryption and `0600`/`0700` permissions are an installation requirement.
- Open: the exact boundaries between `agentd` and separate daemons (who owns scheduling and tunnels).

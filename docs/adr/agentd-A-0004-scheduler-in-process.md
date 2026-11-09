# agentd A-0004. Scheduler inside agentd

Status: accepted

## Context

The spec calls for cron tasks ("summary at 08:00"), delegation ("watch and report"), and event triggers ("email from a given sender goes to a given session"), originally as a separate `agent-sched` daemon. [ADR-0002](0002-db-single-writer.md) left open who owns the schedule. Research recommended moving the schedule into `agentd` and keeping only the observer and the gateway as separate daemons.

Data ([db](../reference/db.md)): `tasks` (`kind` `sched|deleg`, `spec`, `cron`, `tz`, `state`, `persona_id`, `session_id`, `next_run` in UTC), `task_runs` (`running|ok|error|skipped`, retention of the last 200 per task), `triggers` (`source`, `match_json`, `session_id`, `enabled`). Deleting a session pauses its tasks and disables its triggers. Schedule times are expressed in `tasks.tz`, stored in UTC.

If the schedule lived outside `agentd`, `agent-sched` would have to write `task_runs` and events through the core API and start agent turns with permission checks itself, calling the orchestrator over the network: an extra hop and an extra token.

## Decision

1. The scheduler is a **`scheduler` crate inside the `agentd` process**; no `agent-sched` process.
2. **Loop:** one `tokio` task reads the nearest `next_run` among `tasks.state = 'active'`, sleeps until it or until a change signal (create, edit, pause, `run now`), then starts a run. No per-second tick (proposal; the alternative is a polling period, below).
3. **Task run:** the `task_runs` row (`running`) and the recomputed `next_run` (from `cron` in the `tasks.tz` zone) are written by one DB-writer command; the orchestrator then runs an agent turn in the task's session as its persona, with permissions checked like any turn ([A-0005](agentd-A-0005-permissions-engine.md)); the result is `task_runs` (`ok` or `error`) plus a "Tasks" event in the feed (events derive from `task_runs`, see [db](../reference/db.md)).
4. **Permissions without a session:** a run with no chat card (for example `session_id = NULL`) requests permission by event and a `202` response; with no answer within the TTL (10 minutes) the action is denied ([api](../reference/api.md)).
5. **Event triggers:** evaluated when inbound events arrive from the gateway ([architecture](../architecture.md)); a `match_json` match starts a turn in `triggers.session_id`. The `match_json` format is not yet defined.
6. **Concurrency:** runs share the global `max_parallel` limit with ordinary turns; one task never runs concurrently with itself (a repeat while still running is recorded as `task_runs.status = 'skipped'`; proposal).
7. **Time zones:** `next_run` accounts for daylight-saving transitions using a time zone database; the cron and time-zone crates are chosen separately under the dependency policy and are not yet selected.
8. **Missed runs** (core was down at `next_run`): behavior is undefined. Proposal: after start, run each missed task at most once if the delay is below a configurable threshold, otherwise record `skipped` and advance `next_run`; the threshold and setting are not chosen.
9. **Restart:** runs left in `running` after a restart are closed as `error` with the reason "core restarted" (by analogy with `streaming -> interrupted` in [ADR-0002](0002-db-single-writer.md); proposal).

## Consequences

- One process fewer: no separate token, unit, or event-delivery protocol between `agent-sched` and the core.
- A crash of `agentd` also stops the schedule; accepted, since `agentd` is already a single point of failure ([ADR-0002](0002-db-single-writer.md)); restarting the core resumes the loop.
- The original spec (separate `agent-sched`) changes formally and needs a note in the spec and in ADR-0002.
- Permission requests from cron without a session are the least convenient case: the only places a user sees them are the feed event and the banner.
- Scheduler tests use a time abstraction (`Clock` in `core`) so they do not wait on real clocks.

## Alternatives

- **Separate `agent-sched` daemon.** Rejected: an extra process, duplicate access to the DB and orchestrator through the API.
- **systemd timers (`OnCalendar`) per task.** Not examined; user tasks are dynamic, and generating units on the fly complicates rollback and migrations (not verified).
- **System `cron`.** Not examined; time-zone rules and agent permissions would lose the core's single point of control.
- **Polling every second or minute.** Acceptable as a simple implementation but costs more wakeups; may be chosen at implementation time if sleeping until `next_run` proves fragile.

# ADR-0008. Closing the open questions of release 0.1

Status: accepted (the owner may overturn any decision with a separate ADR)

Date: 2026-11-02

## Context

After several reviews a set of open questions remained: the source of Claude quotas, default indicators, project license, encryption of `sessions.db`, retention and volumes, the unofficial WhatsApp channel, who owns `agentd` and by when, and unverified security assumptions. Some of them blocked work (quotas blocked the bar, encryption blocked the first sprint). The decisions below are accepted as working values and are revisited after measurement.

## Decisions

| Question | Decision |
|---|---|
| Source of Claude quotas | In 0.1 the bar shows actual accounting: session and day tokens and cost from the `usage` field of API responses, API-key limits (RPM and tokens) from rate-limit response headers, and GPU/VRAM load of a local model. The remaining subscription window (5 h) is not shown, since no official API exists. "Runs out in" is computed only where a limit with a known remainder and reset exists. Rotating between subscriptions or round-robin was removed from 0.1 (risk of violating provider terms); rotation between API keys of one owner comes after 0.1 |
| Default indicators | Enabled: CPU, RAM, disk, temperature, network (link), AI spend (tokens and cost), unread, `agentd` status. Other groups are off and enabled in Settings |
| Project license | Superseded by [ADR-0011](0011-project-license.md). The dependency policy in `deny.toml` stands: MIT, Apache-2.0, BSD-2/3, ISC, MPL-2.0, Unicode, Zlib and LGPL-2.1+/LGPL-3.0 (GTK, libadwaita, dynamic linking) are allowed; GPL/AGPL and unknown licenses are denied |
| Encryption of `sessions.db` and `index.db` | SQLCipher is not used in 0.1. Mandatory install requirement: disk encryption (LUKS/fscrypt) and file modes `0600`/`0700`; the installer and `--diagnose` check that the user's root partition is encrypted and warn otherwise. Backups are always encrypted ([ADR-0007](0007-distribution-and-backup-keys.md)). Revisit when a multi-user mode appears |
| Retention and volumes | The assumptions in the [DB reference](../reference/db.md) are accepted as working values (events 90 days, `args_json` 90 days, the two latest migration snapshots). Measure on real data, then revisit |
| `tool-results` route, `/health`, `X-Request-Id` | Added to the [API reference](../reference/api.md): `POST /sessions/{id}/tool-results`, an extended `/health`, and an `X-Request-Id` header on all requests and responses |
| `agentd`: ownership and timing | `agentd` is developed in this same repository, under `agentd/`; no separate repository. A minimal real `agentd` (sessions, streaming chat, permissions, audit log) is a precondition for accepting the chat: chat is accepted only against the real core, not against `fake-agentd`. `fake-agentd` remains for tests of the other components |
| Unverified security assumptions | Two time-boxed spikes run before the first sprint. Security: theft of a token by a process of the same uid, file access of an agent under a separate uid via bwrap or systemd-run, WebKitGTK 6.0 sandbox with CSP and user namespaces on Ubuntu 24.04/26.04. Environment: cage + greetd + WebKitGTK + VTE on the target OS, file dialog, clipboard, Cyrillic, HiDPI. Findings so far: a same-uid token is readable; bwrap is blocked by AppArmor while `systemd-run` under a separate uid works; conclusions are reflected in [ADR-0003](0003-api-auth.md). If isolation is not confirmed, modules and agent commands are cut from 0.1 |
| WhatsApp | The channel is an unofficial gateway, at the owner's risk, kept in a module outside the 0.1 GUI; the interface labels it "unofficial channel" |
| Journal "rollback" | Removed from the 0.1 promises; audit remains. Reversible actions (file to trash) become a narrow mechanism later |

## Clarifications

Acceptance of the chat goes against the real minimal `agentd`; other components in 0.1 use `fake-agentd` and contract tests. Further clarifications (GUI token and agent isolation, token kinds, permission-request decisions, quotas in the DB, connection states) are covered in [ADR-0003](0003-api-auth.md) and [ADR-0005](0005-permissions-semantics.md).

## Consequences

- The bar is not blocked: it shows spend and API-key limits.
- Storage encryption is not blocked: disk encryption is an install requirement.
- Chat acceptance depends on the minimal real `agentd`.
- The two spikes precede the first sprint; a failure changes the contents of 0.1.
- The dependency license policy is defined in `deny.toml` but has not been legally reviewed.

## Alternatives

- Scraping the remaining subscription allowance (rejected: fragile and risky under provider terms).
- SQLCipher in 0.1 (rejected: complicates `fake-agentd`, migrations and backup for a single user).
- Accepting all of 0.1 against `fake-agentd` (rejected: proves nothing about the real core).

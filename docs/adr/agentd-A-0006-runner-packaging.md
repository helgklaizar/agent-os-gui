# agentd A-0006. agent-runner as a root service: packaging and prototype findings

Status: accepted

## Context

[ADR-0003](0003-api-auth.md) requires agent commands to run under a dedicated uid, `agentos-run`. A prototype showed that unprivileged `bwrap` does not work on Ubuntu 24.04 (`apparmor_restrict_unprivileged_userns=1`); the working path is a system transient unit via `systemd-run --uid=agentos-run`. `agentd` is a user unit and cannot call system `systemd-run` itself, so a small system helper, `agent-runner`, is needed. Risk: compromise of the helper means root.

Prototype results (`dev-vm`, Ubuntu 24.04.5):

- a root helper with socket activation (`/run/agent-runner.sock`, `Accept=yes`, root), `SO_PEERCRED` check, a directory allowlist, and launch via `systemd-run --uid=agentos-run` with `ProtectSystem=strict`, `ProtectHome=tmpfs` + `BindPaths=<project>`, `IPAddressDeny=any`, `NoNewPrivileges`, `PrivateTmp`, `MemoryMax=512M`, `TasksMax=128`;
- **a project under `/home` works** (needs ACL `x` on the home directory and `rwx` on the project);
- verified: writes to the project; the home directory shows only as the project directory; `~/.ssh` is invisible; `/etc` and `/usr` read-only; `/etc/shadow` denied; network closed (`curl` rc 6); `sudo` denied (`no new privileges`); `/run/user/1000` invisible; directories outside the allowlist and `..` rejected; a directory not owned by the requester rejected;
- **problems:** files created by the agent belong to `agentos-run` (need default ACLs or a group); unclear who maintains the allowlist (polkit or administrator approval); the agent has no network or keys, so `git push` and API calls from commands are impossible (by design: the core does them itself); one uid for all projects; helper compromise means root.

Not covered by the prototype results (unverified): behavior when `agent-runner` is stopped (fail-closed `agentd`), extra `systemd-run` parameters from a request, a very long command, many requests in a row, launch latency (target at most 300 ms), pty, streamed output, Ubuntu 26.04.

## Decision

1. **Composition.** `agent-runner` is a separate binary (the `runner` crate, see [architecture](../architecture.md)), a **system service under root with socket activation**; the client part in `agentd` is the same crate. Minimal dependencies, `unsafe_code = "forbid"`, file at most 300 lines.
2. **Request.** Structured, with a fixed set of fields: project directory, command, timeout, limits. No `systemd-run` parameters come from the request: the runner builds the option set (`-p ...`) from a constant template.
3. **Checks before launch:** `SO_PEERCRED` gives the requester uid; the directory is owned by the requester, contains no `..` after parsing, is not a symlink and has no symlinks pointing outside the project; refuse `/etc`, `~/.ssh`, the agent's own data directory and `/run/user`. Proposal, not verified: refuse if the project contains the agent data directory or equals `$HOME`.
4. **Command unit:** the prototype template above. For projects outside `/home` and `/tmp`, `ProtectHome` and `PrivateTmp` cause no trouble; for projects in `/home`, `ProtectHome=tmpfs` + `BindPaths` and ACLs apply. Choosing between ACLs on the project directory and placing projects under `/srv` is open.
5. **Fail closed.** If `agent-runner` is unavailable, `agentd` does not run commands any other way; the GUI receives `terminal.status = denied` with a reason. This must be a mandatory core test.
6. **Output and lifecycle.** `agentd` shows output to the GUI as `terminal.output` frames (accumulated limit 256 KiB) and `terminal.status` (`started`, `exited`, `killed`, `denied`, `timeout`). Required: streamed output, stop (`systemctl stop` of the transient unit), timeout, exit code. The prototype returned output and exit code; **streamed output and pty were not verified** (the API and ADR-0003 speak of a dedicated pty), so a separate check precedes implementing the agent terminal.
7. **File ownership.** Agent files belong to `agentos-run`; the user must be able to read and edit them. Options: default ACL on the project directory (`setfacl -d`) or a shared group; chosen at implementation, affects `git` in the project. Undecided.
8. **Directory allowlist.** Source and maintenance are undefined (polkit or administrator approval needed). Proposal: the allowlist is extended only by a machine administrator (an `/etc` configuration entry and reload), never by an `agentd` request; "open a new project folder for the agent" then requires privileges, a trade-off against convenience.
9. **One uid for all projects** is an accepted 0.1 limitation: commands from different projects may interfere, and isolation between projects is not promised. Dynamic uids (`DynamicUser=`) are a future proposal, unverified.
10. **Packaging (.deb).** The package installs: the `agent-runner` binary, `agent-runner.socket` and `agent-runner.service` units, the system user `agentos-run` (no login), an AppArmor profile for `agent-shell` and WebKit web processes (needed for the WebKit sandbox, [ADR-0004](0004-modules-sandbox.md), not for the runner), and the requirement `kernel.yama.ptrace_scope=1`. Signing and repository: [ADR-0007](0007-distribution-and-backup-keys.md). `postinst` and `prerm` behavior is not yet described.
11. **Token `kind = agent`.** The unit has no network and no `/run/user`, so the `agentd` socket is unreachable from it. How the agent obtains the core API is undefined ([architecture](../architecture.md)); until then `kind = agent` is unused and agent commands do not call the API.
12. **Multi-user machine.** `agent-runner` is shared; allowlist and `SO_PEERCRED` checks apply per requesting uid; multi-user behavior was not verified.
13. **Target OS.** Verification on Ubuntu 26.04 and comparison with a "Landlock + seccomp without userns" variant have not been done.

## Consequences

- Isolation of agent commands rests on one privileged component; its bug (request forgery, escaping limits) yields root. This calls for a security review and a minimal surface: one socket, a strict format, constant options.
- If the prototype fails on rerun or allows privilege escalation, agent commands (agent terminal, shell) are cut from 0.1 ([ADR-0008](0008-open-questions-closed.md)).
- The agent cannot `git push` or use the network: the core does everything external itself through permissions (`ask` for sending).
- The package must carry the user, units and profile; installing without them leaves agent commands disabled (fail closed).
- VM tests are needed: refusal of a foreign uid, directory outside the allowlist, `..`, a symlink, extra options, a stopped runner, memory and task limits.

## Alternatives

- **Unprivileged `bwrap`.** Rejected: blocked by AppArmor on 24.04; acceptable only with a profile ([ADR-0003](0003-api-auth.md)).
- **`systemd-run --user`.** The user manager cannot launch under another uid; uid isolation would be lost (a process with the same uid reads the token and `environ`).
- **Persistent unit with `DynamicUser=` and `BindPaths=`.** Fallback plan if the prototype failed; not verified.
- **Privileged `agentd`.** Rejected: enlarges the attack surface of the whole core, which reads mail and web pages.
- **`sudo` rule for `systemd-run`.** Not examined; directory and `SO_PEERCRED` checks are harder and misconfiguration risk is higher.
- **Landlock + seccomp without userns.** Proposed by research; not verified.
- **No agent commands in 0.1.** The fallback rule if the prototype fails.

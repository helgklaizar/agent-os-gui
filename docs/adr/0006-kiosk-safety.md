# ADR-0006. Kiosk session safety and startup resilience

Status: accepted

## Context

Per [ADR-0001](0001-init-stack.md), Agent OS runs as a kiosk: after a PAM login the greeter (greetd) starts the cage compositor, which runs the single application `agent-shell`. If `agent-shell` crashes at startup (a GTK or WebKitGTK update, a corrupt layout, an incompatible DB), cage exits and the user is left without a desktop. The only way back was Logout. An operations review named this the main risk: a kiosk with no emergency exit.

The core `agentd` is a separate user unit ([ADR-0002](0002-db-single-writer.md), [ADR-0003](0003-api-auth.md)). The GUI token is handed over through an inherited file descriptor and lives only in GUI process memory; systemd credentials and files are not used. A failure of either unit affects the other. The GUI used to distinguish only "no connection" and "connected", although the causes differ: network, core restart, core fails to start.

## Decision

1. **Restart limit for the kiosk session.** greetd starts the kiosk session through a wrapper that counts abnormal exits of `agent-shell` in a sliding window. Three abnormal exits within 120 seconds count as a session failure: the wrapper does not restart the kiosk but hands control to a fallback session. The counter resets after 120 seconds of stable work.
2. **Automatic switch to a greetd fallback session.** greetd has a second session, "Recovery": a text (TTY) greeter with a ready shell under the same user, without cage and without `agent-shell`. After a session failure (item 1) greetd shows exactly this one, with the failure reason and a diagnostic command. The user can also pick "Recovery" manually in the greeter's session list.
3. **Mandatory package dependency.** The `agent-shell` package has `Depends:` on the package providing the recovery session and wrapper, so a kiosk session cannot be installed without recovery. `prerm` does not remove the recovery session on removal or downgrade of `agent-shell`.
4. **`agentd` units.** The core user unit: `Restart=on-failure`, `RestartSec=2`, `StartLimitIntervalSec=120`, `StartLimitBurst=5`, `WatchdogSec=30` (the core sends `sd_notify WATCHDOG=1`; a hung core is killed and restarted), `OnFailure=agent-notify-failure.service`. The notification unit writes to the journal and creates an event that the GUI shows on the next connection. The `agent-shell` unit has `Restart=on-failure` with the same limit: exceeding it ends the kiosk session and triggers items 1 and 2.
5. **Connection states in the GUI.** One set of states: `connecting`, `connected`, `reconnecting`, `offline`, `core_unavailable`. `core_unavailable` means the GUI sees the socket but the core does not answer `GET /health`, or there is no socket while the `agentd` unit is `failed`/`activating`. The "Core will not start" banner differs from "No connection", offers a "Diagnostics" button (`agent-shell --diagnose`) and does not hide automatically. The GUI tells the cases apart by health answers and by `systemctl --user is-active agentd` over D-Bus. `core_unavailable` is a GUI state; the like-named HTTP error code is called `core_down` to avoid confusing client state with a server response.
6. **DB compatibility.** If the core fails to start because the DB is newer than the code (see [ADR-0002](0002-db-single-writer.md)), it logs the reason and exits with a code that `OnFailure` turns into a "DB newer than the program" event; the GUI shows `core_unavailable` with an explanation.
7. **Tests.** Release acceptance includes an automated test "broken binary leads to reachable fallback": in a VM `agent-shell` is replaced by a binary that always crashes; three launches in a row lead to the Recovery session within 30 seconds of the third failure, and `journalctl --user` and `systemctl --user` work there. A separate test: `kill -9 agentd` five times within 120 seconds gives the unit state `failed` and the "Core will not start" banner, not an eternal "No connection". See [testing.md](../testing.md).

The numeric values (three failures in 120 s, `StartLimitBurst=5`, `WatchdogSec=30`) are accepted starting values, to be refined by tests on real hardware.

## Consequences

- The user can always leave a broken kiosk without an external device: through Recovery on a TTY or through Ctrl+Alt+F3.
- The `.deb` package must carry the wrapper and the recovery session; their installation and testing are part of release acceptance.
- The GUI gets a new connection state and a new dependency on `systemctl --user` over D-Bus; the connection controller and its tests must be extended.
- The restart limit can drop the user into Recovery after a transient failure; the price is a manual session choice in the greeter.
- All numbers and behavior need verification on real greetd on Ubuntu 26.04; until code exists these are decisions, not verified facts.

## Alternatives

- **Only Logout in the greeter.** Does not solve "the GUI crashes at start": a restart loop with no exit.
- **GDM or another full DE as fallback.** More robust but a heavy dependency, and it conflicts with the idea of replacing the desktop.
- **Endless `agent-shell` restart via systemd.** Hides the problem and occupies the screen.
- **An external emergency medium.** Does not scale to a single developer and a user without a second machine.

# ADR-0001. GUI stack

Status: accepted

## Context

Agent OS is a shell on top of Ubuntu: one fullscreen application instead of a desktop, in three layers: OS, AI core (`agentd`), GUI (`agent-shell`). The GUI is a thin client. It knows nothing about models, messengers or mail and talks only to the core API: locally over the unix socket `$XDG_RUNTIME_DIR/agentd.sock`, and over TCP `127.0.0.1:4096` only from outside through a tunnel ([ADR-0003](0003-api-auth.md)). Phones and the CLI use the same API.

The original spec named port 4096 as the primary transport. Open questions were turned into the decision points below.

## Decision

**Rust + GTK4 + libadwaita**, browser on **WebKitGTK**, terminal on **VTE**, run as a user process under **systemd --user**, Wayland session, HTTP + WebSocket transport to the core. The GUI talks to the core only through its API. `agentd` writes `sessions.db`; the GUI stores only layout and theme.

| # | Question | Options | Decision |
|---|---|---|---|
| 1 | Native GTK4 or WebView shell | (a) GTK4 with semi-transparent panels, no blur; (b) HTML/CSS shell like the prototype | (a). `backdrop-filter` glass blur is not available natively in GTK4 and WebKitGTK support is limited and unverified. Blur is secondary; keeping two stacks (GTK + WebView) in one shell costs more. Revisit (b) only if blur becomes mandatory and is verified in WebKitGTK |
| 2 | Kiosk compositor and greeter | cage, labwc; greetd, GDM | cage + greetd: the shortest path from PAM login to a single application. Take labwc if windows and overlay panels are needed |
| 3 | Port `4096` or unix socket | fixed TCP port; per-user unix socket | Unix socket `$XDG_RUNTIME_DIR/agentd.sock` for the local GUI (each user has their own core, no port clashes); TCP `127.0.0.1:4096` only for access from outside through a tunnel. Tokens are mandatory on both transports (the socket is not anonymous, otherwise an agent under the same uid approves its own permission requests); scopes are described in [ADR-0003](0003-api-auth.md) |
| 4 | Chat tabs | single "Chat" tab; several icons | Start with one "Chat" tab; the set is extensible through the module manifest |
| 5 | Quota accounting | 5-hour window as in the prototype; API limits (RPM/tokens) | Account by facts (`usage`, rate-limit headers, GPU/VRAM). The remaining subscription quota is not shown because subscriptions have no official API for it, and no fake forecast is shown. The common format `{window, remaining, reset, burn rate}` allows empty fields ([ADR-0008](0008-open-questions-closed.md)) |
| 6 | Where modules live | WebView panels; native widgets | Module panels are WebViews sandboxed by manifest; the bar and frame stay native, so modules need no shell rebuild. Threat model, separate `WebKitNetworkSession` and bridge allowlist: [ADR-0004](0004-modules-sandbox.md) |

Related ADRs: [ADR-0002](0002-db-single-writer.md) single DB writer, [ADR-0003](0003-api-auth.md) API authentication, [ADR-0004](0004-modules-sandbox.md) module sandbox, [ADR-0005](0005-permissions-semantics.md) permission semantics, [ADR-0006](0006-kiosk-safety.md) kiosk safety, [ADR-0007](0007-distribution-and-backup-keys.md) distribution and backup keys, [ADR-0008](0008-open-questions-closed.md) closed open questions, [ADR-0009](0009-memory-via-mcp.md) memory via MCP (optional).

## Consequences

- The whole GUI depends on the stability of the core API. Loss of connection shows a banner, input is queued, indicators go gray.
- Choice (a) of decision 1 diverges visually from the prototype: blur is replaced by opacity and shadows.
- The prototype remains the reference for layout and behavior, not for code: its bugs (XSS through `innerHTML`, state reset, dead filters) must not be carried over.
- All user-visible strings are escaped; external pages live only inside the WebKit container.

## Spike result: fullscreen under cage

Tested on Ubuntu 24.04 with cage 0.1.5: a fullscreen window, VTE, Cyrillic input and the clipboard work. cage lacks `ext_session_lock_v1` (screen lock), `input_method` (IME) and `wp_fractional_scale`; `zwlr_screencopy` and the virtual keyboard are available to any client of the socket. The greetd to cage chain was not verified on a VM without DRM and VT.

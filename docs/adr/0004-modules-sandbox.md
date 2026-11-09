# ADR-0004. Module threat model and sandbox

Status: accepted

## Context

Per [ADR-0001](0001-init-stack.md), module panels are WebViews while the bar and shell frame stay native. Modules are installed from a marketplace or by URL, and the agent may write them itself. An early draft put the sandbox "on the core side", but a panel's WebView runs inside the GUI process, which the core does not isolate. A security audit found that without a threat model a third-party or agent-written JS app obtains the user's socket or `Origin`, i.e. effectively code execution with user rights. The sandbox must be verified on real WebKit, not only on `fake-agentd`.

## Threat model

| Threat | Vector |
|---|---|
| Token theft and access to the core API | panel JS reads `$XDG_RUNTIME_DIR`, sends requests to the socket or `127.0.0.1` |
| Data leaking out | `fetch`, `XMLHttpRequest`, WebSocket, `<img src>`, forms to an external address |
| XSS and injection into the shell | module responses reach native bar widgets and the session column |
| Reading user files | `file://`, drag-and-drop, file upload |
| Privilege escalation | a module asks for more than its manifest declares; an update silently widens rights |
| Malicious agent-written module | prompt injection leads to a module with excess rights |

Out of scope: compromise of the `agentd` core and of the OS.

## Decision

### WebView isolation

- Each module gets its own `WebKitNetworkSession` (in WebKitGTK 6.0 network settings and the `WebsiteDataManager` belong to it, not to `WebKitWebContext`): separate cookies, storage and cache. Sessions of different modules never overlap, and a module's profile is deleted with the module.
- A module has no access to the `agentd` unix socket, the GUI token in process memory, or an `admin` token ([ADR-0003](0003-api-auth.md)). The WebKitGTK 6.0 web-process sandbox is always on and built on bubblewrap. On Ubuntu 24.04+ `apparmor_restrict_unprivileged_userns=1` can break it, so the package ships an AppArmor profile allowing userns for WebKit processes. Without the profile the sandbox is considered non-working and modules do not open (fail-closed). Disabling the sandbox (`WEBKIT_DISABLE_SANDBOX_THIS_IS_DANGEROUS`) is forbidden.
- Module pages load through the custom scheme `agentmod://<id>/...`; `file://`, `http://127.0.0.1`, `unix:` and arbitrary external schemes are closed.

### CSP and network

- Mandatory policy: `default-src 'self'`, `script-src 'self'`, `connect-src` only from the manifest's `network` list, `frame-ancestors 'none'`, `form-action 'none'`. Inline scripts are forbidden.
- Module network requests go through a `UriSchemeRequest` handler in the GUI that checks the domain against the manifest allowlist; everything else is blocked and logged.

### Bridge module to GUI to agentd

- The only channel is `postMessage` into a bridge with a method allowlist (`bridge.invoke(method, args)`). The bridge checks the method against the module manifest, the argument schema and size, then calls the core API with a `kind = module` token ([ADR-0003](0003-api-auth.md)): scope `chat` or lower, `allowlist_json` from the manifest; `admin` is never granted to modules.
- Bridge results are escaped; a module cannot insert markup into native widgets. A bar widget or session-column button receives only text, a number, an icon from a closed set and a color from the palette, never HTML.

### Rights from the manifest

- The manifest (`id`, `name`, `version`, `entry`, `placement`: panel/widget/button, `permissions`: list of bridge methods, `network`: domains, `fs`: empty or explicit folders) describes everything a module may do.
- Install and update show the list of rights and require confirmation; an update that widens rights is a new request, never silently applied.
- Pre-install checks: signature (if any), package hash, manifest analysis by the observer. An agent-written module is installed only through `ask`.

### Tests

The sandbox is tested on real WebKitGTK, not on a mock: `fetch` to an external address and to `127.0.0.1`, reading `file:///etc/passwd`, access to the socket, a method outside the allowlist, `postMessage` with extra arguments, an inline script under CSP, an XSS string in widget text. Each must be blocked and logged. Bridge contract tests run on `fake-agentd` but do not replace WebKit tests. CI needs a headless Wayland compositor (for example weston). See [testing.md](../testing.md).

## Spike results (Ubuntu 24.04)

- WebKitGTK 6.0's own sandbox does not start with `apparmor_restrict_unprivileged_userns=1` (`bwrap: setting up uid map: Permission denied`). The package must ship an AppArmor profile permitting userns for `agent-shell` and web processes.
- A working CSP set: `default-src 'none'; form-action 'none'` (with `connect-src 'none'`) blocks fetch, beacon, img, WebSocket, forms and `window.open` (the test server received nothing). **Without `form-action 'none'` a form leaks out.**
- **CSP does not cover navigation** (`location.href` reached the server). A `decide_policy` handler rejecting every navigation except the module scheme is mandatory; with it there is no leak. `<meta http-equiv=refresh>` also leaks under CSP and is closed by the same handler.
- `NetworkSession::new_ephemeral()` with a proxy pointing to a nonexistent address is required: without it `link rel=preconnect` opens TCP connections and sends DNS queries that neither CSP nor `decide_policy` see. Network settings live in `NetworkSession`, not `WebContext`.
- WebRTC is unavailable in the build; iframe, `<object>` and prefetch did not leak under CSP, though mixed-content effects are not excluded.

Mandatory scheme for module panels: (1) meta CSP `default-src 'none'; form-action 'none'`; (2) `decide_policy` denying all foreign navigation; (3) ephemeral `NetworkSession` with a dead proxy; (4) AppArmor profiles with `userns` for `bwrap` and the application binary.

## Consequences

- The sandbox responsibility moves from "the core" to the GUI and WebKit.
- Real-WebKit tests in CI and a dedicated security review of modules are required.
- Cost: every module takes a separate WebKit process and memory; the number of simultaneously open panels is limited.
- Native bar widgets are limited to a closed data set, which narrows what modules can do.

## Alternatives

- **All panels in one shared WebView context.** Rejected: one module reads another's data, no profile isolation.
- **Native widgets for all modules.** Rejected per [ADR-0001](0001-init-stack.md) (decision 6): every module would require a shell rebuild.
- **`bwrap` without an AppArmor profile as the main protection.** Rejected: does not work on Ubuntu 24.04+.
- **Modules as separate processes with IPC and no WebView.** Deferred: stronger isolation, but no UI content without a separate framework.
- **Sandbox on the core side only.** Rejected: the core does not control the WebKit process inside the GUI.

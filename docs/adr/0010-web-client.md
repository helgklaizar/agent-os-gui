# ADR-0010. Browser client (`agent-web`)

Status: accepted

Date: 2026-11-04

## Context

The project maintains two GUIs on one core contract: the native GTK4 shell (`agent-shell`, [ADR-0001](0001-init-stack.md)) and a browser client that connects to a live `agentd` on `dev-vm` (Ubuntu) over a tunnel and TCP with a token ([ADR-0003](0003-api-auth.md)). Priority: the `agentd` core and the browser client first, native GTK4 after. The browser version does not depend on GTK, cage, greetd, VTE or WebKitGTK; it needs `agentd` (TCP, WebSocket, token, static serving) and a frontend. Like the GTK shell, it knows nothing about models or messengers: everything goes through the [API reference](../reference/api.md).

Facts behind the decision:

- The `controllers` crate has no GTK and depends only on `serde`, `serde_json`, `fluent-bundle`, `thiserror` and `unic-langid` (about 4k lines: bus, forms, Markdown into a safe model, thresholds, quota forecast, i18n, session tree, chat). `cargo check --locked -p controllers --target wasm32-unknown-unknown` passes without code changes. The transport (`hyper`, unix socket) lives in `core-client`, not in `controllers`.
- `agentd` already has a TCP listener on `127.0.0.1:4096` (off by default, flag `--tcp`), `allowed_hosts` and `allowed_origins` lists, `Origin`/`Host` checks over TCP (`403 forbidden_origin`, WebSocket close code `4403`), `Bearer` tokens, a ban on tokens in the URL, and routes for sessions, messages, permissions, quotas and settings. It lacks static serving, a browser-compatible WebSocket token, a token-issuing CLI, and some bar and event topics.
- A browser `WebSocket` cannot set an `Authorization` header, while the contract allows the token only in a header and forbids it in the URL. Without a decision the browser cannot connect to the WebSocket.
- Under ADR-0003 an external token is at most `chat`, and deciding a permission request requires `admin` and the local socket. A browser arrives over TCP, so without a separate decision it cannot answer an `ask` card.
- The stack policy forbids any layer outside the allowed-stack table without an ADR, and forbids a web framework in the shell; licenses follow `deny.toml` (MIT, Apache-2.0 and similar; GPL denied).
- The original HTML prototype inserted user text without escaping (XSS); this must not be carried over.

## Decision

### 1. Stack: Rust to WebAssembly, Leptos (client-side rendering), built with `trunk`

- A new workspace member `agent-web/` (application crate; logic comes from `controllers`, UI is Leptos in client-side render mode, no SSR). Target `wasm32-unknown-unknown`, built with `trunk`; artifacts (HTML, wasm, JS glue, CSS) are static files.
- **Shared with GTK: the whole `controllers` crate, with no copying:** Markdown model, escaping, quota thresholds and colors, forecast, session tree and inheritance, forms, outbox queue, i18n, request and response types. The GTK-versus-browser split runs only through two layers: transport (`core-client` on `hyper` versus `agent-web` on `fetch` and `WebSocket` via `web-sys`) and view (`ui` on GTK versus `agent-web` on Leptos). Any behavior that can be expressed in `controllers` lives there, not in a view.
- **One language and one set of checks:** `cargo fmt`, `clippy -D warnings`, `cargo deny`, `--locked`, and the `controllers` tests cover both versions. No new package manager (npm) is introduced, so the supply-chain surface stays in `Cargo.lock` and `deny.toml`.
- **Licenses:** `leptos`, `wasm-bindgen`, `web-sys`, `js-sys` and `trunk` are MIT and/or Apache-2.0. `deny.toml` gets the target `wasm32-unknown-unknown`.
- **Budget and fallback (assumption):** wasm plus glue at most 2 MiB gzipped. If the budget fails, or Leptos blocks accessibility, fall back to a hybrid: `controllers` compiled to a wasm module exposing functions (Markdown, thresholds, forecast) with the view written in a light JS framework; the boundary is `wasm-bindgen`. The fallback changes neither the core nor this ADR, only this section.

### 2. XSS and page security

- User and external text reaches the DOM only as text nodes through the typed `controllers::markdown` model, which cannot express raw HTML. `inner_html`, `outer_html`, `insert_adjacent_html`, `document.write`, `eval` and dynamic `<script>` are forbidden, enforced by `clippy.toml` (`disallowed-methods`) and a CI check.
- Links: only `http`, `https` and `mailto` schemes (anything else is plain text), `rel="noopener noreferrer"`, external navigation with a warning.
- `agentd` sends a CSP with the static files: `default-src 'none'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self'; img-src 'self' data:; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'`. Deviations (for example Leptos inline styles) are recorded with a reason, not added silently. Also `X-Content-Type-Options: nosniff`, `Referrer-Policy: no-referrer`, `Cross-Origin-Opener-Policy: same-origin`.
- The "evil strings" test set `controllers::testkit::evil_strings` also runs through the DOM in browser tests.

### 3. Delivery: `agentd` serves the static files itself

- `agentd` serves the built directory (`--web-root PATH`; in the package, the install directory) at `GET /` and `GET /app/*` on the same port as the API. Page and API share one origin: no CORS in production, no cookies, `connect-src 'self'`, one version of client and API in one package, one port in the tunnel.
- Static files are anonymous (they are code, not user data): the only exception to the rule that only `GET /health` is anonymous (ADR-0003). Protection: response size and type limits, no escaping the directory (`..`, outward symlinks), `ETag`, `Cache-Control: no-cache` for HTML and `immutable` for hashed file names. Data still requires a token.
- **CORS for development only** (`trunk serve` on another port): the flag `--dev-cors ORIGIN` answers preflight `OPTIONS` for listed origins and allows the headers `Authorization`, `Content-Type`, `X-Request-Id`, `If-Match`; it is off in production and `Access-Control-Allow-Origin: *` is never used. The preferred dev mode is a `trunk` proxy to the same origin, which needs no CORS at all.
- A separate static server (nginx, Caddy, Pages) was rejected for 0.1: two origins mean CORS and a second allow-list, version drift, a second tunnel port and an extra operated component.

### 4. Access to `dev-vm` and authentication

- **Transport.** TCP listens only on `127.0.0.1:4096` on the VM (never on an external interface). From the workstation: `ssh -N -L 4096:127.0.0.1:4096 dev-vm`, then the browser opens `http://127.0.0.1:4096`. Loopback over HTTP is a secure context for browsers. A public tunnel terminates TLS itself; its address is added to `allowed_hosts` and `allowed_origins`.
- **Origin and Host.** The page's own origin (scheme, host and port from `Host`, if the host is in `allowed_hosts`) is allowed automatically; anything else must be in `allowed_origins`. A page from a foreign origin gets `403 forbidden_origin`, a WebSocket gets `4403`.
- **Token.** Same format (`agt_` + base64url, only SHA-256 stored in the DB). Issued on the VM by a local command, `agentd token issue --kind external --scope chat --label <label> --ttl <days>` (the value is printed once); login is a token entry form. A token in the URL is forbidden.
- **Browser storage.** In tab memory by default; an optional "remember for this tab" uses `sessionStorage` (cleared when the tab closes). `localStorage`, cookies and IndexedDB are forbidden for the token. The XSS protection in section 2 is the main barrier: a token leaked from the page equals `chat` access.
- **WebSocket from a browser.** The token travels in `Sec-WebSocket-Protocol: agentd.v1, bearer.<token>`; the core validates it, answers with the protocol `agentd.v1` and never echoes the token value. The base64url alphabet is valid for subprotocol tokens. Non-browser clients keep the `Authorization` header. A URL without a token and close codes `4401`, `4403` are unchanged. This extends the [API reference](../reference/api.md) and ADR-0003.
- **Permission decisions from a browser.** ADR-0003 forbids deciding permission requests over TCP; for the browser an exception is made: a new token kind `web` with scope `decide` = `chat` + `POST /permission-requests/{id}/decision` (without `permissions`, `secrets`, `files`, `logout` or token issuing). Default lifetime 7 days (maximum 30), issued only locally, every decision is written to `actions_log` with the token id, and the request shows its source ([ADR-0005](0005-permissions-semantics.md)). Risk: a leaked token of this kind can confirm agent actions; mitigated by the short lifetime, revocation (`4401`) and by dangerous routes staying behind local `admin`. Until implemented, the web client shows `ask` cards read-only.
- Over TCP, `logout`, `files`, `secrets`, `permissions` and token issuing remain unavailable: Settings pages with secrets and permissions are read-only or absent in the browser.

### 5. What `agentd` needs for the browser

| Item | Purpose |
|---|---|
| Static serving, `--web-root`, `/` and `/app/*` | page from the same origin |
| Security headers and CSP | XSS and clickjacking protection |
| Token via `Sec-WebSocket-Protocol`, echo only `agentd.v1` | WebSocket from a browser |
| `agentd token issue/list/revoke` (local) | first token on the VM without a GUI |
| TCP enabled by configuration, automatic own origin, `allowed_origins` list | tunnel to `dev-vm` |
| `--dev-cors` (development only) | `trunk serve` on another port |
| Tests: Origin/Host over TCP, token in WebSocket, static files without directory escape | security boundary; no surviving mutants in token and Origin checks |
| `agentd` unit on `dev-vm`, tunnel, how-to | live acceptance point |
| Contract updates (ADR-0003, API reference, OpenAPI) | contract matches code |

Data for screens comes from existing and planned core work: the first live version uses what exists (sessions, streaming messages, quotas); bar topics, events and notifications, search and the agent terminal come with later core work.

### 6. Implemented state

- `agentd`: static serving (`--web-root`, `/` and `/app/*`, `ETag`, `Cache-Control: no-cache` for all files, directory escape and outward symlinks return `404`), CSP and security headers, WebSocket token from `Sec-WebSocket-Protocol`, automatic own origin, `--allow-host`, `--allow-origin`, `--dev-cors`; an explicit `--tcp` enables the listener.
- **Web token with decision right:** kind `web` and scope `decide` are introduced by a DB migration (an earlier `@decide` grant in `allowlist_json` was removed). `kind = web`, `scope = decide`, lifetime mandatory: 7 days default, 30 maximum. Issued by `POST /v1/tokens` with `decide: true` or `agentd token issue --decide` (local, `admin`). The single exception: `POST /permission-requests/{id}/decision` over TCP, one-shot (`allow`/`deny`); "Always" returns `403 always_needs_gui`; other `admin` and local-only routes stay closed.
- Browser: `ask` and observer cards with buttons, an outbox queue on disconnect (retry by `client_id`), a notification feed on the `events` topic, a read-only Settings window without secrets.
- `agent-web` client: token login (memory, optional `sessionStorage`), bar (link, quotas), session tree, streaming chat through `ChatController`, `SessionTreeController` and `BarCluster` from `controllers`; `controllers` gained `time::Instant` (on `wasm32` this is `web-time`).
- Schedule, search and agent terminal windows: schedule (`GET /tasks`, pause and resume, run, history), search (`GET /search`, Ctrl+K) and a view-only agent terminal (session `terminal.*` frames). **xterm.js is not used:** it inserts a `<style>` element that the page CSP (`style-src 'self'`) blocks, which lost colors and sizes in Chromium, and weakening `style-src` to `unsafe-inline` for a terminal was rejected. A custom ANSI parser (`agent-web/src/term.rs`) is used instead: SGR colors via CSS classes, other control sequences dropped, 256 KiB limit. Using xterm.js remains acceptable if the CSP changes.
- Browser scenarios (Playwright, headless Chromium) live in `agent-web/tests/e2e/` and run through `agent-web/scripts/e2e.sh`; their dependencies are installed into a cache outside the repository and are not part of cargo or `cargo deny`.
- Deviation from section 1: an external `boot.js` starts the wasm (`agent-web/scripts/build.sh` extracts the inline script `trunk` emits), because a CSP without `unsafe-inline` would otherwise block the page. Release wasm size is about 0.85 MiB uncompressed.
- `deny.toml`: added the target `wasm32-unknown-unknown`, the permissive Boost license `BSL-1.0` (comes with `xxhash-rust` through `leptos`) and an exception for advisory `RUSTSEC-2024-0436` (`paste` is unmaintained; status only, not a vulnerability), to be revisited on 2027-02-04.

## Consequences

- Native GTK and the browser evolve on the same `controllers`: fixing threshold logic or escaping fixes both. The cost is that `controllers` must stay free of GTK, `tokio` and platform dependencies; `cargo check --target wasm32-unknown-unknown -p controllers` belongs in CI.
- A new layer in the stack (Rust/wasm, Leptos, `trunk`) and a lifted ban on web frameworks for `agent-web` only; neither `agent-shell` nor `fake-agentd` take a framework.
- `agentd` gains a public surface (static files, TCP, browser WebSocket). Risks: a token in the browser, TCP exposed by a configuration mistake, drift between client and core versions.
- Limits of the web client versus GTK: no embedded tool browser (foreign pages in a frame hit `X-Frame-Options` and CSP) and no VTE; the agent terminal is view-only. Shell functions (kiosk, session logout, Recovery) make no sense in a browser.
- Accessibility and i18n are built in from the start (`aria` roles, focus, Fluent translations).
- `fake-agentd` stays for development and view tests; acceptance of the web client happens on a real `agentd` on `dev-vm`.

## Alternatives

- **TypeScript + Preact/Svelte, client from OpenAPI.** Rejected: duplicates `controllers` logic (Markdown, thresholds, forecast, queue, inheritance) in a second language with permanent drift, a second test suite and a second supply chain (npm) outside `cargo deny`. Upsides: more developers and ready components, smaller wasm. It returns as the hybrid fallback in section 1.
- **Hybrid from the start: `controllers` as a wasm module, view in JS.** Rejected initially: two worlds and a `wasm-bindgen` boundary on every screen; kept as a fallback.
- **Server-side rendering, HTMX.** Rejected: the WebSocket stream, outbox queue and connection state fit the client better; the core should not know about markup.
- **Tauri, Electron.** Not a browser: packaging and updating for the same view; out of scope.
- **Separate static server or CDN.** Rejected for 0.1 (section 3).
- **Token in an `HttpOnly` cookie or in the URL.** Rejected: a cookie opens CSRF and changes the ADR-0003 model; the URL is forbidden. The subprotocol variant stays.
- **First WebSocket message carrying the token instead of a subprotocol.** An acceptable replacement if the subprotocol does not pass through the tunnel; to be decided by testing through SSH and a public tunnel.

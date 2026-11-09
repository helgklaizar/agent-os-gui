# Getting started

Status: pre-alpha (0.0.1-alpha). Commands below were checked against the workspace manifests and the command-line parsers in the code. Everything listens on `127.0.0.1` or a unix socket only; do not expose these ports.

## 1. Prerequisites

| Tool | Notes |
|---|---|
| Rust 1.99.1 | pinned in `rust-toolchain.toml` (components `clippy`, `rustfmt`); `rustup` picks it automatically |
| C toolchain | `build-essential`, `pkg-config`, `libssl-dev` |
| SQLite | system library: `libsqlite3-dev` (the code links the system SQLite, it is not bundled) |
| Linux | `agentd`, `fake-agentd`, `agent-web` and the client crates build on Linux; the GTK window (`agent-shell`) builds and runs on Linux only (on other systems the `ui` crate compiles as an empty skeleton and the binary exits with a message) |

Additional packages for the native GTK shell (Ubuntu 24.04 package names, verified versions in parentheses):

```bash
sudo apt-get install libgtk-4-dev libadwaita-1-dev \
  libwebkitgtk-6.0-dev libvte-2.91-gtk4-dev        # GTK 4.14, libadwaita 1.5, WebKitGTK 2.52, VTE 0.76
sudo apt-get install weston                         # headless Wayland, only for UI scenario tests
```

For the browser client:

```bash
rustup target add wasm32-unknown-unknown
cargo install --locked trunk
```

Optional tools used by the checks ([testing](testing.md)): `cargo install --locked cargo-deny cargo-mutants`.

A development VM (referred to as `dev-vm` in these docs) with Ubuntu, the packages above and Rust is the recommended environment for the full run. Agent command isolation additionally needs `kernel.yama.ptrace_scope=1` and a root-run runner service; see the [security model](reference/security-model.md).

## 2. Build

From the repository root:

```bash
cargo build --locked --workspace
```

Binaries (in `target/debug/`):

| Binary | Package | Purpose |
|---|---|---|
| `agentd` | `agentd` | the core daemon and its subcommands |
| `fake-agentd` | `fake-agentd` | scripted mock of the core for GUI development and tests |
| `agent-shell` | `ui` | the native GTK4 window (Linux only) |

Build only the core: `cargo build --locked -p agentd`.

## 3. Run the mock core (`fake-agentd`)

`fake-agentd` implements the API from [api.md](reference/api.md) with a scripted agent, so you can develop a client without a real core.

```bash
cargo run --locked -p fake-agentd -- \
  --socket "$XDG_RUNTIME_DIR/fake-agentd.sock" \
  --tcp 127.0.0.1:0 \
  --token-file /tmp/fake-agentd.token \
  --scenario fake-agentd/scenarios/idle.toml \
  --clock real
```

It prints `socket <path>` and `tcp <addr>` and runs until interrupted. Flags: `--socket`, `--tcp`, `--token-file` (writes the `gui` token to a file; for tests and manual work only), `--scenario`, `--clock manual|real`, `--db`, `--reply-delay-ms`. Scenarios live in `fake-agentd/scenarios/*.toml`.

To start the GTK window against the mock (Linux, test-token feature enabled, the token comes from a file instead of the production descriptor):

```bash
AGENTD_SOCKET="$XDG_RUNTIME_DIR/fake-agentd.sock" \
AGENTD_TOKEN_FILE=/tmp/fake-agentd.token \
cargo run --locked -p ui --features test-token-file --bin agent-shell
```

`agent-shell --smoke` shows the window, waits for the first frame and exits with code 0 (code 1 after 10 seconds).

## 4. Run the real core (`agentd`)

```bash
cargo run --locked -p agentd -- serve \
  --db /tmp/sessions.db \
  --socket "$XDG_RUNTIME_DIR/agentd.sock" \
  --tcp 127.0.0.1:4096
```

Defaults when flags are omitted: database `~/.agentos/sessions.db` (or `AGENTD_DB`), socket `$XDG_RUNTIME_DIR/agentd.sock` (or `AGENTD_SOCKET`), TCP is off unless `--tcp` or `AGENTD_TCP` is set. Migrations are applied at startup. Other subcommands (a bad argument prints the full usage): `migrate [--check]`, `token issue|list|revoke`, `gui-launch`, `agent-launch`, `backup`, `backup-verify`, `restore`, `runner`, `watch`, `--diagnose`.

Model providers need an API key. Keys are written through the API (`PUT /secrets/{ref}`), never read back and never stored in the database; see [api.md](reference/api.md) and the [security model](reference/security-model.md).

### Issue a token

Every connection needs a token, including the unix socket. With the core running:

```bash
cargo run --locked -p agentd -- token issue --name web --scope chat --ttl-days 1 \
  --socket "$XDG_RUNTIME_DIR/agentd.sock"
```

The raw value is printed once and only its SHA-256 is stored. Add `--decide` to allow the token to decide permission requests. `token list` and `token revoke --id N` manage existing tokens.

In the intended product flow the GUI never sees a token file: `agentd gui-launch -- <command>` issues an `admin` token and passes it to the GUI as an inherited descriptor ([ADR-0003](adr/0003-api-auth.md)).

## 5. Browser client (`agent-web`)

```bash
agent-web/scripts/build.sh --release     # output: agent-web/dist/, served under /app/
agent-web/scripts/build.sh --debug       # unoptimized build
```

Requirements: the `wasm32-unknown-unknown` target, `trunk`, `python3`. The build moves the inline bootstrap script into `boot.js` because the page uses a strict content security policy.

Serve it from the core:

```bash
cargo run --locked -p agentd -- serve --db /tmp/sessions.db \
  --socket "$XDG_RUNTIME_DIR/agentd.sock" --tcp 127.0.0.1:4096 --web-root agent-web/dist
# open http://127.0.0.1:4096/app/
```

or use the dev server, which proxies `/v1` and the WebSocket to `127.0.0.1:4096` (no CORS needed):

```bash
cd agent-web && trunk serve              # http://127.0.0.1:5173/app/
```

If the core rejects the page origin, start `agentd` with `--allow-origin http://127.0.0.1:5173`. Other web-related `serve` flags: `--allow-host`, `--dev-cors`.

## 6. Connect

Open the page, paste the token from section 4 into the sign-in form. The token lives in page memory (optionally `sessionStorage` for the tab) and is never put into a URL; a token in a URL is rejected with `401`.

To reach the core from another device, publish `127.0.0.1:4096` through your own tunnel (WireGuard, SSH, a reverse proxy) and add the tunnel host with `--allow-host` and the page origin with `--allow-origin`. Failed attempts are limited to 5 per minute per address. Do not bind the port to a public interface.

## 7. Verify the setup

```bash
cargo test --locked --workspace          # see testing.md for narrower commands
curl -s http://127.0.0.1:4096/v1/health  # anonymous health endpoint
```

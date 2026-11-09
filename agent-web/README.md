# agent-web

Browser client for Agent OS. A Rust to WebAssembly app (Leptos) that talks to a local or remote `agentd` over REST and WebSocket. All client logic comes from the `controllers` crate; this crate holds only the transport and the view. The UI is English only.

The token lives in page memory (optionally in `sessionStorage` for the tab) and is never put into a URL. The page CSP is strict (`script-src 'self' 'wasm-unsafe-eval'`, `style-src 'self'`), so the build moves the inline bootstrap script into `boot.js`.

## Build

Requirements: Rust (see `rust-toolchain.toml`), the `wasm32-unknown-unknown` target, [`trunk`](https://trunkrs.dev), `python3`.

```bash
rustup target add wasm32-unknown-unknown
cargo install --locked trunk

agent-web/scripts/build.sh --release   # output: agent-web/dist/ (served under /app/)
agent-web/scripts/build.sh --debug     # unoptimized build
agent-web/scripts/package.sh           # agent-web/pkg/agent-web-<version>.tar.gz
```

Unit tests run on the host: `cargo test --locked -p agent-web`. Lints for the wasm target:
`cargo clippy --locked -p agent-web --target wasm32-unknown-unknown --all-targets -- -D warnings`.

## Run locally

1. Start a local `agentd` with TCP on `127.0.0.1:4096` and issue a web token:

   ```bash
   agentd serve --db /tmp/sessions.db --socket "$XDG_RUNTIME_DIR/agentd.sock" \
     --tcp 127.0.0.1:4096 --web-root agent-web/dist
   agentd token issue --name web --scope chat --ttl-days 1 --socket "$XDG_RUNTIME_DIR/agentd.sock"
   ```

   Add `--decide` to the token command for a token that may decide permission requests.

2. Either open the page served by `agentd` itself (`http://127.0.0.1:4096/app/` after a build), or run the dev server, which proxies `/v1` and the WebSocket to `127.0.0.1:4096` so no CORS is needed:

   ```bash
   cd agent-web && trunk serve        # http://127.0.0.1:5173/app/
   ```

   If the core rejects the page origin, start `agentd` with `--allow-origin http://127.0.0.1:5173`.

3. Paste the token into the sign-in form.

## End-to-end tests (Playwright)

The scenarios in `tests/e2e/specs` run against a live `agentd` (chromium, firefox, webkit). Tokens come from the environment and are never printed.

```bash
export AGENT_WEB_TOKEN=<chat token>
export AGENT_WEB_TOKEN_DECIDE=<chat token issued with --decide>   # optional: permission and settings scenarios
export AGENT_WEB_URL=http://127.0.0.1:4096                        # default
export AGENT_WEB_BROWSERS=chromium                                # default: chromium,firefox,webkit
agent-web/scripts/e2e.sh                 # installs Playwright into ~/.cache/agent-web-e2e and runs the specs
agent-web/scripts/e2e.sh -g "Settings"   # extra arguments go to `playwright test`
```

Some scenarios skip themselves without their variables: the terminal and 429-limit scenarios need a second `agentd` with a runner (`AGENT_WEB_URL_TERM`, `AGENT_WEB_TOKEN_TERM`, `AGENT_WEB_PROJECT_TERM`), and the token revocation scenario needs `AGENT_WEB_REVOKE=1` with ssh access to the dev VM.

### Dev VM helpers

`scripts/dev-vm-*.sh`, `scripts/e2e-dev-vm.sh` and `scripts/e2e-on-vm.sh` deploy `agentd` plus the client to a VM over ssh and run the full suite there. They are generic: set `DEV_VM_HOST` to an ssh host (required, no default); optional `DEV_VM_DEST` (deploy directory under the VM home, default `agent-web-stand`) and `TERM_STAND_DIR` (VM-side work directory of the terminal stand, default `/tmp/agent-web-term-stand`).

```bash
export DEV_VM_HOST=my-vm
agent-web/scripts/dev-vm-deploy.sh                       # build, rsync, build agentd on the VM, start the unit
ssh -N -L 4096:127.0.0.1:4096 -L 4097:127.0.0.1:4097 "$DEV_VM_HOST" &
agent-web/scripts/e2e-dev-vm.sh                          # tokens, terminal stand, Playwright in all browsers
```

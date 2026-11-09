# GitHub repository settings (copy-paste)

Nothing here has been executed. Run the commands yourself after review, from a clone of this repository, authenticated with `gh auth login` as `helgklaizar`.

## About

Description (EN, 208 characters, limit 350):

```
Fullscreen shell on Linux for working with AI agents: status bar plus four columns (notifications, sessions, chat, tools). Thin client of the agentd core; native GTK4/libadwaita and Rust/WASM browser clients.
```

Website: leave empty.

Topics (14): `rust`, `gtk4`, `libadwaita`, `wasm`, `webassembly`, `leptos`, `ai-agents`, `llm`, `linux`, `desktop`, `kiosk`, `agent-os`, `mcp`, `sqlite`.

## Create the repository and push (the repo must exist before the push)

```bash
gh repo create helgklaizar/agent-os-gui --public \
  --description "Fullscreen shell on Linux for working with AI agents: status bar plus four columns (notifications, sessions, chat, tools). Thin client of the agentd core; native GTK4/libadwaita and Rust/WASM browser clients." \
  --disable-wiki --license apache-2.0 --source . --remote origin
```

If the repository is created from this tree, drop `--license apache-2.0` (the tree already has `LICENSE`) and push explicitly:

```bash
git remote add origin git@github.com:helgklaizar/agent-os-gui.git
git push -u origin main
```

## Settings

```bash
gh repo edit helgklaizar/agent-os-gui \
  --description "Fullscreen shell on Linux for working with AI agents: status bar plus four columns (notifications, sessions, chat, tools). Thin client of the agentd core; native GTK4/libadwaita and Rust/WASM browser clients." \
  --homepage "" \
  --enable-issues --enable-discussions \
  --enable-wiki=false --enable-projects=false \
  --add-topic rust --add-topic gtk4 --add-topic libadwaita --add-topic wasm --add-topic webassembly \
  --add-topic leptos --add-topic ai-agents --add-topic llm --add-topic linux --add-topic desktop \
  --add-topic kiosk --add-topic agent-os --add-topic mcp --add-topic sqlite

# Private vulnerability reporting (Security -> Advisories) is required by SECURITY.md:
gh api -X PUT repos/helgklaizar/agent-os-gui/private-vulnerability-reporting
```

## Tag and pre-release

```bash
git tag -a v0.0.1-alpha -m "Agent OS GUI 0.0.1-alpha"
git push origin v0.0.1-alpha

gh release create v0.0.1-alpha --prerelease --title "v0.0.1-alpha" --notes "$(cat <<'EOF'
First public alpha of Agent OS GUI.

**Not everything works and the API is not stable.** Expect breaking changes to the HTTP/WebSocket contract, the database schema and the settings format.

Included:
- `agentd`: core daemon (REST + WebSocket, scoped tokens, sessions, chat with streaming, permission engine, observer, events, MCP client, scheduler, search and memory, encrypted backup, `.deb` packaging script)
- `agent-shell`: native GTK4/libadwaita client (Linux)
- `agent-web`: browser client (Rust/WASM)
- `fake-agentd`: scripted mock core for client development and tests

Not included or incomplete: module panel sandbox, full kiosk-session validation, manual accessibility passes, signed packages, languages other than English.

See docs/roadmap.md and docs/getting-started.md. License: Apache-2.0.
EOF
)"
```

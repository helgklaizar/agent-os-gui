# Contributing

Thanks for your interest. The project is pre-alpha (0.0.1-alpha): the API, database schema and settings format can still change, so please open an issue to discuss larger changes before writing code.

## Setup

- Rust 1.99.1 (pinned in `rust-toolchain.toml`; `rustup` installs it automatically).
- Optional tools: `cargo-deny`, `gitleaks`, `trunk` and the `wasm32-unknown-unknown` target for the browser client. See [docs/getting-started.md](docs/getting-started.md).
- Enable the secret guard hook once per clone: `git config core.hooksPath .githooks`.

## Before you open a pull request

Run the project check; it must pass:

```bash
bash scripts/check.sh              # fmt, clippy -D warnings, tests, cargo-deny, gitleaks
CHECK_STRICT=1 bash scripts/check.sh   # missing cargo-deny or gitleaks becomes a failure (CI mode)
```

The GTK client builds on Linux only; on other systems its crate compiles as an empty skeleton. If you change Linux-only code, say how you verified it.

## Rules

- Tests come with the change. Fix a bug with a test that fails first. Do not weaken or delete tests to make a change pass.
- No `unsafe` code, no `unwrap` outside tests (workspace lints enforce this), no mocks in product code (use `fake-agentd` in tests).
- Any user-visible text in the clients is escaped; never insert HTML into the page (the web client forbids it via clippy rules).
- Interface strings live in `assets/i18n/en.ftl`, not in code.
- New dependencies must pass `cargo deny` (permissive licenses, rustls only).
- Never commit secrets, tokens, real hostnames, IP addresses or personal data, not even in tests or docs. Use `example.com`, documentation IP ranges (`192.0.2.0/24`) and `user`.
- Keep commits focused; describe what and why. Documentation changes go in `docs/`.

## Security

Do not report vulnerabilities in public issues. See [SECURITY.md](SECURITY.md).

## License

By contributing you agree that your contribution is licensed under the Apache License 2.0 (see [LICENSE](LICENSE)).

# Security Policy

Agent OS GUI is pre-alpha software (0.0.1-alpha). It is not recommended for production use.

## Reporting a vulnerability

Please do not open a public issue for a vulnerability or a leaked secret.
Use GitHub private vulnerability reporting: **Security -> Advisories -> Report a vulnerability** on this repository.
Expect an initial answer within a few working days.

## Scope

The GUI (`agent-shell`, `agent-web`), the core daemon `agentd`, the mock core `fake-agentd`, scripts and hooks in this repository.

## Design notes

- Secrets are not stored in the repository. `.githooks/pre-commit` runs three layers: built-in key patterns, a ban on tracked files excluded by `.gitignore`, and gitleaks. Enable it with `git config core.hooksPath .githooks`.
- API tokens are stored only as SHA-256 hashes; the value is shown once at issue time (see `docs/adr/0003-api-auth.md`). Provider secrets go through a write-only `PUT /secrets/{ref}`; the database keeps only a `secret_ref`.
- A token readable by any process of the same uid cannot be protected by `SO_PEERCRED`; the GUI token is therefore passed via a file descriptor and agent commands run under a separate uid (see `docs/reference/security-model.md`).
- The secret-detection regex lives in `scripts/secret-pattern.sh`; `scripts/check.sh` also runs gitleaks.
- A leaked key must be rotated immediately; removing it from history alone is not a fix.

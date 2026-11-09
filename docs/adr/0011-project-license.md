# ADR-0011. Project license

Status: accepted

Date: 2026-11-09

## Context

The project is pre-alpha (0.0.1-alpha) and is being published. It needs an explicit license. Requirements: permissive terms, an explicit patent grant for contributors and users, and compatibility with the dependency license allow-list in `deny.toml` (MIT, Apache-2.0, BSD, ISC, MPL-2.0, Zlib, Unicode, LGPL for dynamically linked GTK libraries).

## Decision

The project is licensed under Apache-2.0.

- It is permissive and carries an express patent grant with patent-retaliation, which MIT and BSD lack.
- It is compatible with every license on the `deny.toml` allow-list, so dependencies can be combined into one distributable without relicensing.
- Contributions are accepted under the same license (Apache-2.0 section 5).

## Consequences

- A `LICENSE` file with the full Apache-2.0 text and a `NOTICE` file with the project attribution live at the repository root.
- Every Cargo manifest (workspace and members) sets `license = "Apache-2.0"` (via `workspace.package` where possible).
- `cargo deny` keeps checking dependencies; GPL/AGPL and unknown licenses stay denied.
- Redistributors must retain the license, the `NOTICE` and notices of changes.

## Alternatives

- **MIT.** Simpler, but no patent grant.
- **GPL-3.0 / AGPL-3.0.** Copyleft would conflict with the permissive dependency policy and limit adoption.
- **Proprietary.** Incompatible with publishing the source.

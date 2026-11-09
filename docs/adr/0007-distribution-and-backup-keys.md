# ADR-0007. Package distribution and a single backup and key scheme

Status: accepted

Date: 2026-11-02

Not implemented yet. The package repository, the signing keys and `agent-backup` do not exist. Decisions marked "(assumption)" are accepted here and will be confirmed by the implementation.

## Context

An operations review found no update channel, no package signing, no rollback procedure and no single description of backup keys. Backup was described by two diverging texts:

| Source | Key | Restore |
|---|---|---|
| DB reference (before the fix) | public `age` key (recipient) stored in `settings`, private key kept off the machine, passphrase only for manual exports | "delete `sessions.db-wal` and `-shm`" |
| Release plan | passphrase from the keyring (libsecret) plus a one-time recovery key | `-wal`/`-shm` are moved to `*.bak-YYYYMMDD`, not deleted |

Further problems: the project forbids deleting data (only move to trash or to a backup); keys for package signing were not described anywhere; backup verification amounted to a test unpack rather than a real restore; losing the private key makes every backup unreadable.

## Decision

### 1. Reconciling the two backup texts

- **Base: recipient-based `age` encryption.** Archives are encrypted to public keys (recipients); private keys are not stored on the machine. This is the only scheme in which an unattended backup needs no secret on disk and does not fail when nobody is at the screen.
- **A second recipient, the recovery key, is mandatory.** It is issued once at setup and not kept next to the archive. A copy of the archive goes to external media. `keys.env` never enters a backup in clear text.
- **The keyring passphrase is used only for manual exports** (`POST /sessions/{id}/export`, manual backup) in `age` scrypt mode. It is not the key of the automatic backup.
- **Restore moves `-wal`/`-shm` to `*.bak-YYYYMMDD`**; nothing is deleted.

### 2. Backup keys (`age`)

- Two key pairs are created at the first `agent-backup` setup: a primary key (for the owner) and a recovery key (on a second, independent medium). Public parts are stored in `settings` (`backup.recipient.primary`, `backup.recipient.recovery`; key names are an assumption). Private parts are handed to the owner once and are not kept on the machine: password manager, paper or hardware token.
- Private keys never reach a backup, a log, a chat or documentation. Documents name secrets and where they live, never their values.
- **Rotation:** every 12 months or on suspected compromise (assumption). The new primary key is added as a recipient; old archives stay readable by the old key until retention expires, so the old private key is kept until the date of the last archive it encrypted. After rotation a new recovery key is issued; the old one is destroyed only after a successful restore test with the new one.
- **Emergency access:** loss of the primary key falls back to the recovery key. Losing both makes backups unreadable; this is an accepted and recorded risk, which is why the monthly test below is mandatory.

### 3. Monthly restore test

- Once a month (assumption: first Sunday) the owner runs `agent-backup restore-test` on a separate test machine (not the user's workstation and not a CI runner): full restore into a disposable VM or container, decryption with the recovery key, unpack, `PRAGMA integrity_check` and `foreign_key_check`, start `agentd` on the copy, check that `PRAGMA user_version` matches `schema_migrations`.
- Private key material is entered by the owner from its medium for the duration of the test and is never stored on the workstation or a CI runner. Everything except key entry is automated, so the test is semi-automatic rather than nightly.
- The result is an event in the feed and a "Backup" indicator: green or red, showing the age of the last successful restore (not just of the last backup). Red if no restore has passed for more than 35 days (assumption).
- This complements the automatic archive check, which only proves that an archive unpacks; the monthly test proves a system can be brought up from it.
- Two physically separate storage locations are required; an archive on the same disk does not count as a backup.

### 4. Package distribution

- **Own apt repository**, tool `reprepro` (assumption; `aptly` is the alternative if repository snapshots are needed). The repository lives on a host separate from the workstation and from the CI runner.
- **GPG signing:** repository metadata (`Release`/`InRelease`) is signed with a dedicated release-signing key, distinct from the `age` keys. The private signing key lives only on a separate signing machine and is not available to the CI runner: the runner builds packages and hands artifacts over for signing. The public part is published in the repository and in the [getting-started guide](../getting-started.md).
- **Signing key rotation:** the key is valid for 2 years and renewed in advance. On compromise it is revoked (the revocation certificate is stored separately), a new key is published, and clients receive a key-update package with instructions.
- **Channels:** suites `stable` and `beta`; each Ubuntu release gets its own component or version suffix (`~ubuntu24.04`, `~ubuntu26.04`). The default install uses `stable`.
- **Delivery:** the client adds the repository under `/etc/apt/sources.list.d/` with `signed-by=` pointing at the signing key and updates with plain `apt`. Automatic application through `unattended-upgrades` is off by default (the GUI must not be updated in the middle of a dialog); an "updates ready" indicator in the bar leads to a manual confirmation (assumption).
- **Rollback:** `apt install agent-shell=<version>` plus restoring the `pre-migration` snapshot if the schema changed. The repository keeps at least the three latest releases. Restoring a snapshot discards everything written after the update, so the current DB is first moved to `~/backup/post-update-DATE/` (not deleted) and the GUI reports the data-loss window (from `pre-migration` to the rollback). A rollback without a schema change leaves data untouched.
- **Key custody by a second person:** the private `age` keys and the release-signing key are kept in two places: the primary one and a sealed copy with a person designated by the owner (offline medium, no password in chat). The copy is checked quarterly by trial-decrypting a test backup. Until a custodian is designated this is an open risk, not a met requirement.
- **External heartbeat:** `agentd` and the backup service ping a dead-man-switch monitoring service every 5 minutes; 15 minutes without a ping notifies the owner off the machine. Monitoring that lives on the same machine cannot notice that machine going down.
- **Build and tag** come only from CI; metadata signing happens on the signing machine.

### 5. Backup and release monitoring

After a successful backup, a successful `restore-test` and a release, `agent-backup` and the release process ping the monitoring service. A missing ping beyond the deadline (35 days for `restore-test`, an assumption) raises an alarm. The check URL and ping key are secrets, stored through `PUT /secrets/{ref}` ([ADR-0003](0003-api-auth.md)) and never written in documents.

## Consequences

- One backup and key scheme replaces two contradictory texts.
- Losing both private keys remains unrecoverable; this is mitigated by the monthly restore test and two independent media.
- An apt repository host and a signing procedure are needed; until they exist the package is installed manually with `dpkg -i` and updated by hand, without signatures. A release without a signed repository is marked as limited.
- Extra work: `agent-backup` with recipient keys, a restore test in a disposable environment, repository publication, key rotation.
- Disk encryption is a separate mandatory install requirement (see [ADR-0008](0008-open-questions-closed.md)).

## Alternatives

- **Passphrase from the keyring only, for the automatic backup.** Rejected: the passphrase must sit on the machine; losing it or changing the keyring makes backups unreadable; restoring on another machine is impossible without the key.
- **Full-disk encryption instead of archive encryption.** Does not replace backup (the archive leaves for external media) and does not protect copies; stays a separate requirement.
- **`aptly` instead of `reprepro`.** More powerful (snapshots, mirrors) but heavier for one maintainer; acceptable if the number of channels grows.
- **Publishing only as release assets (no apt repository).** Simpler, but there is no signed metadata, and `apt upgrade` and version rollback become manual; acceptable as a temporary measure.
- **PPA / Launchpad.** Requires publicity and an account; unsuitable for a private repository.
- **Auto-update through `unattended-upgrades`.** Rejected as the default because it can interrupt a running session.

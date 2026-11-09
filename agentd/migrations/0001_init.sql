CREATE TABLE schema_migrations (
  version     INTEGER PRIMARY KEY,
  name        TEXT NOT NULL,
  applied_at  INTEGER NOT NULL
);

CREATE TABLE settings (
  key         TEXT PRIMARY KEY,          -- theme, language, memory.source, balance.*, browser.bookmarks.<session_id>, tunnels.api_external…
  value_json  TEXT NOT NULL,
  rev         INTEGER NOT NULL DEFAULT 1, -- grows on every write; the client sends If-Match, a mismatch gives 412 (no header: 428), so GUI and phone do not overwrite each other
  updated_at  INTEGER NOT NULL
) WITHOUT ROWID;

CREATE TABLE api_token (
  id             INTEGER PRIMARY KEY AUTOINCREMENT,
  token_hash     TEXT NOT NULL UNIQUE
                 CHECK (length(token_hash) = 64 AND token_hash NOT GLOB '*[^0-9a-f]*'), -- SHA-256 of the raw token, hex
  label          TEXT NOT NULL CHECK (length(label) BETWEEN 1 AND 100),                -- "GUI", "phone", "CLI", "agent", module name
  scope          TEXT NOT NULL CHECK (scope IN ('read','chat','admin')),
  kind           TEXT NOT NULL DEFAULT 'external' CHECK (kind IN ('gui','external','test','agent','module')),
  allowlist_json TEXT CHECK (allowlist_json IS NULL OR json_valid(allowlist_json)),     -- allowed methods; required for agent and module
  created_by     INTEGER REFERENCES api_token(id) ON DELETE SET NULL,                  -- who issued it (admin token)
  created_at     INTEGER NOT NULL,
  expires_at     INTEGER,                                                              -- NULL for the GUI token (lives as long as the GUI process)
  revoked_at     INTEGER,                                                              -- revocation: the value never changes
  last_used_at   INTEGER,
  CHECK (expires_at IS NULL OR expires_at > created_at),
  CHECK (kind <> 'external' OR scope <> 'admin'),                                      -- an external client never gets admin (ADR-0003)
  CHECK (kind NOT IN ('agent','module') OR scope <> 'admin'),                          -- agent and module never get admin
  CHECK (kind NOT IN ('agent','module') OR allowlist_json IS NOT NULL)                 -- they always have a list of allowed methods
);
CREATE INDEX idx_api_token_active ON api_token(token_hash) WHERE revoked_at IS NULL;

CREATE TRIGGER trg_api_token_no_unrevoke BEFORE UPDATE OF revoked_at ON api_token
WHEN OLD.revoked_at IS NOT NULL AND NEW.revoked_at IS NOT OLD.revoked_at
BEGIN SELECT RAISE(ABORT, 'api_token: revoked_at cannot be changed'); END;

CREATE TRIGGER trg_api_token_hash_immutable BEFORE UPDATE OF token_hash ON api_token
BEGIN SELECT RAISE(ABORT, 'api_token: token_hash cannot be changed'); END;

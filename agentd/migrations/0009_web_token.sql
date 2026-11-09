-- 0009_web_token: token kind `web` and scope `decide` for the browser client (ADR-0003).
-- Scope `decide` = `chat` + a one-off decision on a permission request; kind `web` is a browser token (lifetime up to 30 days, never `admin`).
-- The CHECK of `api_token` cannot be altered, so the table is recreated (the runner turns `foreign_keys` off during the migration).

CREATE TABLE api_token_new (
  id             INTEGER PRIMARY KEY AUTOINCREMENT,
  token_hash     TEXT NOT NULL UNIQUE
                 CHECK (length(token_hash) = 64 AND token_hash NOT GLOB '*[^0-9a-f]*'),
  label          TEXT NOT NULL CHECK (length(label) BETWEEN 1 AND 100),
  scope          TEXT NOT NULL CHECK (scope IN ('read','chat','decide','admin')),
  kind           TEXT NOT NULL DEFAULT 'external' CHECK (kind IN ('gui','external','test','agent','module','web')),
  allowlist_json TEXT CHECK (allowlist_json IS NULL OR json_valid(allowlist_json)),
  created_by     INTEGER REFERENCES api_token(id) ON DELETE SET NULL,
  created_at     INTEGER NOT NULL,
  expires_at     INTEGER,
  revoked_at     INTEGER,
  last_used_at   INTEGER,
  CHECK (expires_at IS NULL OR expires_at > created_at),
  CHECK (kind <> 'external' OR scope <> 'admin'),
  CHECK (kind NOT IN ('agent','module') OR scope <> 'admin'),
  CHECK (kind NOT IN ('agent','module') OR allowlist_json IS NOT NULL),
  CHECK (kind <> 'web' OR scope IN ('chat','decide')),                 -- a browser client never gets admin
  CHECK (scope <> 'decide' OR kind IN ('web','test')),                 -- `decide` only for the web token (and the test kind)
  CHECK (kind <> 'web' OR expires_at IS NOT NULL)                      -- a web token always has an expiry
);

INSERT INTO api_token_new (id, token_hash, label, scope, kind, allowlist_json, created_by, created_at, expires_at, revoked_at, last_used_at)
SELECT id, token_hash, label, scope, kind, allowlist_json, created_by, created_at, expires_at, revoked_at, last_used_at FROM api_token;

DROP TABLE api_token;
ALTER TABLE api_token_new RENAME TO api_token;

CREATE INDEX idx_api_token_active ON api_token(token_hash) WHERE revoked_at IS NULL;

CREATE TRIGGER trg_api_token_no_unrevoke BEFORE UPDATE OF revoked_at ON api_token
WHEN OLD.revoked_at IS NOT NULL AND NEW.revoked_at IS NOT OLD.revoked_at
BEGIN SELECT RAISE(ABORT, 'api_token: revoked_at cannot be changed'); END;

CREATE TRIGGER trg_api_token_hash_immutable BEFORE UPDATE OF token_hash ON api_token
BEGIN SELECT RAISE(ABORT, 'api_token: token_hash cannot be changed'); END;

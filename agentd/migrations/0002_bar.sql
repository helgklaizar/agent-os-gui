CREATE TABLE accounts (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  slug        TEXT NOT NULL UNIQUE,     -- stable name from the prototype: cl1, wa1, gm2...; exposed as key in the API
  type        TEXT NOT NULL,            -- wa|tg|gmail|anthropic|ollama|…
  label       TEXT NOT NULL,            -- «Claude · key-1»
  secret_ref  TEXT,                     -- variable name in keys.env / keyring entry; the secret itself is not stored
  status      TEXT NOT NULL DEFAULT 'disconnected' CHECK (status IN ('connected','disconnected','qr','error')),
  created_at  INTEGER NOT NULL,
  deleted_at  INTEGER                   -- soft delete: gateway messages remain (section 5)
);

CREATE TABLE quotas (
  account_id     INTEGER PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
  kind           TEXT NOT NULL DEFAULT 'api_key' CHECK (kind IN ('api_key','local')),
  window_h       REAL,                      -- limit window, hours; NULL = no window (local model, limit unknown)
  used           REAL CHECK (used IS NULL OR used BETWEEN 0 AND 1),        -- fraction of the limit used; NULL = limit unknown
  rate           REAL,                      -- burn rate, x of the allowed rate; NULL = cannot be computed
  load           REAL CHECK (load IS NULL OR load BETWEEN 0 AND 1),        -- GPU/VRAM load of a local model (kind = local)
  usage_tokens   INTEGER,                   -- tokens per window from the usage field of API responses
  usage_cost     REAL,                      -- cost per window
  limit_requests INTEGER,                   -- request limit (rate-limit headers of the API key)
  limit_tokens   INTEGER,                   -- token limit
  remaining      REAL CHECK (remaining IS NULL OR remaining BETWEEN 0 AND 1), -- remaining limit, fraction; NULL = unknown
  reset_at       INTEGER,
  updated_at     INTEGER NOT NULL
);

CREATE TABLE quota_samples (
  account_id    INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  ts            INTEGER NOT NULL,
  used          REAL,                       -- NULL if the limit is unknown
  usage_tokens  INTEGER,
  cost          REAL,                      -- cost, if known
  PRIMARY KEY (account_id, ts)
) WITHOUT ROWID;

CREATE TABLE indicators (
  id          TEXT PRIMARY KEY,          -- cpu, quota.cl1 (accounts.slug), whapi, backup…
  enabled     INTEGER NOT NULL DEFAULT 1,
  position    INTEGER NOT NULL DEFAULT 0,
  config_json TEXT NOT NULL DEFAULT '{}', -- thresholds, period, click_action
  last_state  TEXT NOT NULL DEFAULT 'unknown' CHECK (last_state IN ('ok','warn','bad','nodata','unknown')),
  transition_seq INTEGER NOT NULL DEFAULT 0, -- transition number; part of events.dedup_key
  last_change_at INTEGER
);

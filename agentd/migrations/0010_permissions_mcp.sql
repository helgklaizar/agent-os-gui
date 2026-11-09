-- 0010_permissions_mcp: permission requests and the MCP server registry (docs/reference/db.md 3.14, D46.8/D46.9). DDL verbatim from db.md.

CREATE TABLE permission_requests (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  actions_id  INTEGER REFERENCES actions_log(id) ON DELETE SET NULL,
  session_id  INTEGER REFERENCES sessions(id) ON DELETE CASCADE,
  action      TEXT NOT NULL,
  options_json TEXT NOT NULL,            -- allow_once|allow_always|reject_once|reject_always
  status      TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending','allowed','denied','expired','cancelled')),
  decided_by  TEXT,                      -- token id or 'timeout'
  decided_at  INTEGER,
  expires_at  INTEGER NOT NULL,
  created_at  INTEGER NOT NULL
);
CREATE INDEX idx_permreq_pending ON permission_requests(status, expires_at);

CREATE TABLE mcp_servers (
  id        INTEGER PRIMARY KEY AUTOINCREMENT,
  name      TEXT NOT NULL UNIQUE,
  transport TEXT NOT NULL CHECK (transport IN ('stdio','http')),
  command   TEXT,
  args_json TEXT,
  env_ref   TEXT,                        -- reference to secrets, values are not stored
  enabled   INTEGER NOT NULL DEFAULT 1
);

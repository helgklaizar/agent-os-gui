-- Migration 0004: chat. DDL is a snapshot of db.md sections 3.2, 3.6, 3.7 and 3.11 (observer_findings); db.md is the source of truth.
CREATE TABLE messages (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  session_id  INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  client_id   TEXT,                     -- idempotency of client sends (UUID); NULL for agent messages
  role        TEXT NOT NULL CHECK (role IN ('user','agent','system','tool')),
  author      TEXT,                     -- "Agent · Persona"
  body_md     TEXT NOT NULL,            -- Markdown, escaped on output
  status      TEXT NOT NULL DEFAULT 'done' CHECK (status IN ('streaming','done','stopped','error','interrupted')),
  meta_json   TEXT,                     -- permission cards (pending/approved/denied, resolved_by, resolved_at), watcher confirmations, attachments
  ts          INTEGER NOT NULL
);
CREATE INDEX idx_messages_session ON messages(session_id, id);
CREATE UNIQUE INDEX idx_messages_client ON messages(session_id, client_id) WHERE client_id IS NOT NULL;

CREATE TABLE permissions (
  scope       TEXT NOT NULL CHECK (scope IN ('global','persona','session')),
  scope_id    INTEGER NOT NULL DEFAULT 0,        -- 0 for global; no FK on personas/sessions, triggers clean up (section 3.7)
  action      TEXT NOT NULL,                     -- read|send|delete|pay|shell|browse|memory_write|config_write
  mode        TEXT NOT NULL CHECK (mode IN ('allow','ask','deny')),
  PRIMARY KEY (scope, scope_id, action)
) WITHOUT ROWID;

CREATE TABLE actions_log (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  ts          INTEGER NOT NULL,
  actor       TEXT NOT NULL CHECK (actor IN ('agent','cron','user','observer','module')),
  session_id  INTEGER REFERENCES sessions(id) ON DELETE SET NULL,
  persona_id  INTEGER REFERENCES personas(id) ON DELETE SET NULL,
  action      TEXT NOT NULL,
  target      TEXT,
  args_json   TEXT,                      -- no secrets, values are masked; zeroed by retention and on session deletion
  decision    TEXT CHECK (decision IN ('allow','ask-approved','ask-denied','deny')),
  result      TEXT,
  reversible  INTEGER NOT NULL DEFAULT 0,
  undo_json   TEXT,                      -- how to undo; NULL when reversible = 0 (sending, payment)
  undone_at   INTEGER
);
CREATE INDEX idx_actions_ts      ON actions_log(ts DESC);
CREATE INDEX idx_actions_session ON actions_log(session_id, ts);

-- Session deletion: remove its permissions, zero the log arguments, drop browser bookmarks
CREATE TRIGGER trg_sessions_cleanup BEFORE DELETE ON sessions
BEGIN
  DELETE FROM permissions WHERE scope = 'session' AND scope_id = OLD.id;
  UPDATE actions_log SET args_json = NULL WHERE session_id = OLD.id;
  DELETE FROM settings WHERE key = 'browser.bookmarks.' || OLD.id;
END;

-- Hard persona deletion (after export): remove its permissions
CREATE TRIGGER trg_personas_cleanup AFTER DELETE ON personas
BEGIN
  DELETE FROM permissions WHERE scope = 'persona' AND scope_id = OLD.id;
END;

CREATE TABLE observer_findings (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  ts          INTEGER NOT NULL,
  kind        TEXT NOT NULL,             -- injection|phishing|dangerous_cmd|module|permission_loosen
  severity    TEXT NOT NULL CHECK (severity IN ('low','med','high')),
  ref_table   TEXT,                      -- gw_messages|actions_log|modules|permissions (no FK)
  ref_id      TEXT,                      -- row id as text (modules have a string key)
  verdict     TEXT NOT NULL CHECK (verdict IN ('blocked','asked','allowed')),
  detail      TEXT
);
CREATE INDEX idx_observer_ts ON observer_findings(ts DESC);

CREATE TRIGGER trg_findings_ref_actions AFTER DELETE ON actions_log
BEGIN
  UPDATE observer_findings SET ref_id = NULL WHERE ref_table = 'actions_log' AND ref_id = CAST(OLD.id AS TEXT);
END;

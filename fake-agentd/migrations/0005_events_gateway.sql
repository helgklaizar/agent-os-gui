-- Migration 0005: events and gateway. DDL is a snapshot of db.md sections 3.3 and 3.11 as of 2026-11-02; db.md is the source of truth.
CREATE TABLE events (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  kind        TEXT NOT NULL CHECK (kind IN ('ai','task','msg')),
  sub         TEXT NOT NULL,            -- ai: agent|watch|claude|local; task: sched|deleg|done; msg: sys|wa|tg|mail (core data, not an enum in the schema)
  level       TEXT NOT NULL DEFAULT 'info' CHECK (level IN ('info','warn','error','ok')),
  text        TEXT NOT NULL,
  session_id  INTEGER REFERENCES sessions(id) ON DELETE SET NULL,  -- no session_id = a system event, lives independently
  source      TEXT,                     -- source daemon: agentd, agent-gw, agent-sys...
  dedup_key   TEXT,                     -- <indicator>:<transition_seq>: one state transition = one event
  unread      INTEGER NOT NULL DEFAULT 1,
  ts          INTEGER NOT NULL
);
CREATE INDEX idx_events_filter  ON events(kind, sub, unread, ts DESC);
CREATE INDEX idx_events_session ON events(session_id, unread);
CREATE UNIQUE INDEX idx_events_dedup ON events(dedup_key) WHERE dedup_key IS NOT NULL;

CREATE TABLE contacts (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  name        TEXT NOT NULL,
  handles_json TEXT NOT NULL DEFAULT '{}',  -- {wa:…, tg:…, mail:…}
  persona_id  INTEGER REFERENCES personas(id) ON DELETE SET NULL
);

CREATE TABLE gw_messages (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  account_id  INTEGER NOT NULL REFERENCES accounts(id) ON DELETE RESTRICT, -- mail and chat archive is not cascaded
  ext_id      TEXT NOT NULL,             -- id in the external system
  direction   TEXT NOT NULL CHECK (direction IN ('in','out')),
  contact_id  INTEGER REFERENCES contacts(id) ON DELETE SET NULL,
  body        TEXT NOT NULL,
  routed_session_id INTEGER REFERENCES sessions(id) ON DELETE SET NULL,  -- routing by rules
  isolated    INTEGER NOT NULL DEFAULT 0,  -- watcher isolated it (injection, phishing)
  read        INTEGER NOT NULL DEFAULT 0,
  ts          INTEGER NOT NULL,
  UNIQUE (account_id, ext_id)
);
CREATE INDEX idx_gw_unread ON gw_messages(account_id, read, ts DESC);

-- Links without FK must not dangle on deleted rows
CREATE TRIGGER trg_findings_ref_gw AFTER DELETE ON gw_messages
BEGIN
  UPDATE observer_findings SET ref_id = NULL WHERE ref_table = 'gw_messages' AND ref_id = CAST(OLD.id AS TEXT);
END;

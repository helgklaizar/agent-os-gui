-- Migration 0003: session tree. DDL is a snapshot of db.md sections 3.1 and 3.4; db.md is the source of truth.
CREATE TABLE personas (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  name        TEXT NOT NULL,
  color       TEXT NOT NULL,
  model_acc   INTEGER REFERENCES accounts(id) ON DELETE SET NULL,  -- anthropic|ollama
  style       TEXT NOT NULL DEFAULT '',
  lang        TEXT NOT NULL DEFAULT 'ru',
  signature   TEXT NOT NULL DEFAULT '',
  mem_mode    TEXT NOT NULL DEFAULT 'own' CHECK (mem_mode IN ('own','shared','none')),
  notif_mode  TEXT NOT NULL DEFAULT 'all' CHECK (notif_mode IN ('all','important','digest','off')),
  work_hours  TEXT NOT NULL DEFAULT '',
  quiet_hours TEXT NOT NULL DEFAULT '',
  deleted_at  INTEGER                   -- soft delete: personal memory does not move to shared (section 5)
);

CREATE TABLE persona_accounts (
  persona_id  INTEGER NOT NULL REFERENCES personas(id) ON DELETE CASCADE,
  account_id  INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  PRIMARY KEY (persona_id, account_id)
);

CREATE TABLE folders (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  parent_id   INTEGER REFERENCES folders(id) ON DELETE SET NULL,  -- folder deletion: contents move one level up (implemented by agentd)
  name        TEXT NOT NULL,
  persona_id  INTEGER REFERENCES personas(id) ON DELETE SET NULL, -- inherited dynamically by nested sessions
  open        INTEGER NOT NULL DEFAULT 1,
  created_at  INTEGER NOT NULL,
  CHECK (parent_id IS NULL OR parent_id <> id)
);

CREATE TABLE sessions (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  folder_id   INTEGER REFERENCES folders(id) ON DELETE SET NULL,
  parent_id   INTEGER REFERENCES sessions(id) ON DELETE CASCADE,  -- NULL = session, not NULL = thread (one level)
  name        TEXT NOT NULL,
  persona_id  INTEGER REFERENCES personas(id) ON DELETE SET NULL, -- NULL = as the folder / default
  path        TEXT,                                               -- local folder: agent, terminal, files
  cfg_json    TEXT NOT NULL DEFAULT '{}',                         -- model, perms, mem, notif, ctx, auto
  unread      INTEGER NOT NULL DEFAULT 0,                         -- badge (field b of the prototype)
  archived    INTEGER NOT NULL DEFAULT 0,
  created_at  INTEGER NOT NULL,
  updated_at  INTEGER NOT NULL,
  CHECK (parent_id IS NULL OR parent_id <> id)
);
CREATE INDEX idx_sessions_folder ON sessions(folder_id);
CREATE INDEX idx_sessions_parent ON sessions(parent_id);
CREATE INDEX idx_folders_parent  ON folders(parent_id);

-- Folders: no cycles and no deeper than 8 levels
CREATE TRIGGER trg_folders_no_cycle BEFORE UPDATE OF parent_id ON folders
WHEN NEW.parent_id IS NOT NULL
BEGIN
  SELECT RAISE(ABORT, 'folder cycle') WHERE NEW.parent_id = NEW.id OR EXISTS (
    WITH RECURSIVE up(id, d) AS (
      SELECT NEW.parent_id, 1
      UNION ALL SELECT f.parent_id, up.d + 1 FROM folders f JOIN up ON f.id = up.id
        WHERE f.parent_id IS NOT NULL AND up.d < 32
    ) SELECT 1 FROM up WHERE id = NEW.id);
END;

CREATE TRIGGER trg_folders_depth_ins BEFORE INSERT ON folders
WHEN NEW.parent_id IS NOT NULL
BEGIN
  SELECT RAISE(ABORT, 'folder depth') WHERE EXISTS (
    WITH RECURSIVE up(id, d) AS (
      SELECT NEW.parent_id, 1
      UNION ALL SELECT f.parent_id, up.d + 1 FROM folders f JOIN up ON f.id = up.id
        WHERE f.parent_id IS NOT NULL AND up.d < 32
    ) SELECT 1 FROM up WHERE d >= 8);
END;

CREATE TRIGGER trg_folders_depth_upd BEFORE UPDATE OF parent_id ON folders
WHEN NEW.parent_id IS NOT NULL
BEGIN
  SELECT RAISE(ABORT, 'folder depth') WHERE EXISTS (
    WITH RECURSIVE up(id, d) AS (
      SELECT NEW.parent_id, 1
      UNION ALL SELECT f.parent_id, up.d + 1 FROM folders f JOIN up ON f.id = up.id
        WHERE f.parent_id IS NOT NULL AND up.d < 32
    ) SELECT 1 FROM up WHERE d >= 8);
END;

-- Threads: only one level (a thread does not create a thread)
CREATE TRIGGER trg_sessions_thread_ins BEFORE INSERT ON sessions
WHEN NEW.parent_id IS NOT NULL
BEGIN
  SELECT RAISE(ABORT, 'thread depth') WHERE EXISTS (
    SELECT 1 FROM sessions p WHERE p.id = NEW.parent_id AND p.parent_id IS NOT NULL);
END;

CREATE TRIGGER trg_sessions_thread_upd BEFORE UPDATE OF parent_id ON sessions
WHEN NEW.parent_id IS NOT NULL
BEGIN
  SELECT RAISE(ABORT, 'thread depth') WHERE EXISTS (
    SELECT 1 FROM sessions p WHERE p.id = NEW.parent_id AND p.parent_id IS NOT NULL);
END;

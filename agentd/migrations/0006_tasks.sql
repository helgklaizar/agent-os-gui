-- 0006_tasks: tasks, runs, triggers (docs/reference/db.md 3.8).

CREATE TABLE tasks (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  kind        TEXT NOT NULL CHECK (kind IN ('sched','deleg')),
  title       TEXT NOT NULL,
  spec        TEXT NOT NULL,             -- prompt or command
  cron        TEXT,                      -- time in timezone tz
  tz          TEXT NOT NULL DEFAULT 'Asia/Jerusalem',
  state       TEXT NOT NULL DEFAULT 'active' CHECK (state IN ('active','paused','done')),
  persona_id  INTEGER REFERENCES personas(id) ON DELETE SET NULL,
  session_id  INTEGER REFERENCES sessions(id) ON DELETE SET NULL,
  next_run    INTEGER,                   -- UTC
  created_at  INTEGER NOT NULL
);

CREATE INDEX idx_tasks_next ON tasks(state, next_run);

CREATE TABLE task_runs (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  task_id     INTEGER NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
  started_at  INTEGER NOT NULL,
  finished_at INTEGER,
  status      TEXT NOT NULL CHECK (status IN ('running','ok','error','skipped')),
  output      TEXT
);

CREATE INDEX idx_task_runs_task ON task_runs(task_id, started_at DESC);

CREATE TABLE triggers (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  source      TEXT NOT NULL,             -- mail|wa|tg|git|…
  match_json  TEXT NOT NULL,             -- "mail from Alex"
  session_id  INTEGER REFERENCES sessions(id) ON DELETE SET NULL,
  enabled     INTEGER NOT NULL DEFAULT 1
);

-- Session deletion does not silently erase rules: tasks are paused, triggers are disabled (then the FK nulls session_id)
CREATE TRIGGER trg_sessions_pause_tasks BEFORE DELETE ON sessions
BEGIN
  UPDATE tasks    SET state = 'paused' WHERE session_id = OLD.id AND state = 'active';
  UPDATE triggers SET enabled = 0      WHERE session_id = OLD.id;
END;

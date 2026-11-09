-- 0008_memory_facts: memory facts (docs/reference/db.md 3.10). Hard deletion of a persona is blocked while facts exist (D33).

CREATE TABLE memory_facts (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  persona_id  INTEGER REFERENCES personas(id) ON DELETE RESTRICT, -- NULL = shared memory; personal memory cannot be lost by cascade
  session_id  INTEGER REFERENCES sessions(id) ON DELETE SET NULL, -- where the fact came from
  text        TEXT NOT NULL,
  source      TEXT NOT NULL DEFAULT 'agent' CHECK (source IN ('agent','user')),
  ts          INTEGER NOT NULL
);
CREATE INDEX idx_memory_persona ON memory_facts(persona_id);

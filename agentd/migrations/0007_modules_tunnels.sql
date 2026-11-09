-- 0007_modules_tunnels: modules and tunnels (docs/reference/db.md 3.9), trigger for finding references to modules (3.11).

CREATE TABLE modules (
  id            TEXT PRIMARY KEY,
  name          TEXT NOT NULL,
  manifest_json TEXT NOT NULL,           -- permissions, network, files (sandbox), placement
  installed     INTEGER NOT NULL DEFAULT 0,
  enabled       INTEGER NOT NULL DEFAULT 1,
  placement     TEXT NOT NULL DEFAULT 'panel' CHECK (placement IN ('panel','widget','button')),
  config_json   TEXT NOT NULL DEFAULT '{}'
);

CREATE TABLE tunnels (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  type        TEXT NOT NULL CHECK (type IN ('wireguard','tailscale','ssh','cloudflare','ngrok')),
  addr        TEXT NOT NULL,
  descr       TEXT NOT NULL DEFAULT '',
  enabled     INTEGER NOT NULL DEFAULT 0
);

-- References without FK must not dangle on deleted rows (module key is a string)
CREATE TRIGGER trg_findings_ref_modules AFTER DELETE ON modules
BEGIN
  UPDATE observer_findings SET ref_id = NULL WHERE ref_table = 'modules' AND ref_id = OLD.id;
END;

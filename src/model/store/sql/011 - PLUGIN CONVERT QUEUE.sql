CREATE TABLE plugin_convert_queue (
  id TEXT PRIMARY KEY,
  plugin_id TEXT NOT NULL,
  library_id TEXT NOT NULL,
  media_id TEXT NOT NULL,
  filename TEXT NOT NULL,
  request_json TEXT NOT NULL,
  status TEXT NOT NULL,
  plugin_job_id TEXT,
  progress REAL NOT NULL DEFAULT 0,
  converted_id TEXT,
  error TEXT,
  requested_by TEXT,
  added INTEGER NOT NULL DEFAULT (round((julianday('now') - 2440587.5)*86400.0 * 1000)),
  modified INTEGER NOT NULL DEFAULT (round((julianday('now') - 2440587.5)*86400.0 * 1000))
);

CREATE INDEX plugin_convert_queue_plugin_status
  ON plugin_convert_queue(plugin_id, status, added);

CREATE INDEX plugin_convert_queue_library_status
  ON plugin_convert_queue(library_id, status, added);

CREATE TRIGGER modified_plugin_convert_queue AFTER UPDATE ON plugin_convert_queue
BEGIN
  UPDATE plugin_convert_queue
  SET modified = round((julianday('now') - 2440587.5)*86400.0 * 1000)
  WHERE id = NEW.id;
END;

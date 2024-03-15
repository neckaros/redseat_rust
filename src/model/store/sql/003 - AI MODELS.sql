CREATE TABLE plugins (
  id   TEXT PRIMARY KEY,
  name TEXT    NOT NULL,
  kind TEXT    NOT NULL,
  path TEXT    NOT NULL,
  settings TEXT    NOT NULL,
  libraries TEXT    NOT NULL,
  version INTEGER DEFAULT 1
);
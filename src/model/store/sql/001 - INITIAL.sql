CREATE TABLE Backups (
  id   TEXT PRIMARY KEY,
  source TEXT NOT NULL,
  credentials TEXT,
  library TEXT,
  path TEXT NOT NULL,
  schedule TEXT,
  filter TEXT,
  last INTEGER,
  password TEXT,
  size INTEGER,
  name TEXT NOT NULL);

CREATE TABLE Backups_Files (
  backup   TEXT NOT NULL,
  library TEXT,
  file TEXT NOT NULL,
  id TEXT,
  path TEXT,
  hash TEXT,
  sourcehash TEXT NOT NULL,
  size INTEGER,
  modified INTEGER DEFAULT 0,
  added INTEGER DEFAULT 0,
  iv TEXT,
  thumbsize NUMBER,
  infoSize NUMBER,
  error TEXT,
  FOREIGN KEY (backup) REFERENCES Backups(id),
  FOREIGN KEY (library) REFERENCES Libraries(id),
  PRIMARY KEY (backup, library, file, sourcehash)
);

CREATE TABLE Credentials (
  id   TEXT PRIMARY KEY,
  name TEXT    NOT NULL,
  source TEXT    NOT NULL,
  type TEXT    NOT NULL,
  login TEXT    NOT NULL,
  password TEXT    NOT NULL,
  preferences TEXT    NOT NULL
, user_ref TEXT, refreshtoken TEXT, expires INTEGER);

CREATE TABLE Invitation (
  code   TEXT PRIMARY KEY,
  role TEXT    NOT NULL,
  expires INTEGER
, library TEXT);

CREATE TABLE Libraries (
  id   TEXT PRIMARY KEY,
  name TEXT    NOT NULL,
  type TEXT    NOT NULL,
  source TEXT    NOT NULL,
  root TEXT    NOT NULL,
  settings TEXT    NOT NULL,
  crypt INTEGER DEFAULT 0,
  credentials TEXT ,
  plugin TEXT
);

CREATE TABLE Libraries_Users_Rights (
  user_ref TEXT,
  library_ref TEXT,
  roles TEXT,
  PRIMARY KEY (user_ref, library_ref));

CREATE TABLE Users (
  id   TEXT PRIMARY KEY,
  name TEXT    NOT NULL,
  role TEXT    NOT NULL,
  preferences TEXT    NOT NULL
);

CREATE TABLE Watched (
  type TEXT    NOT NULL,
  id TEXT    NOT NULL,
  user_ref TEXT NOT NULL   , 
  date INTEGER DEFAULT 0, 
  'modified' INTEGER DEFAULT 0,
  PRIMARY KEY (type, id, user_ref)
);

CREATE TABLE "migrations" (
  id   INTEGER PRIMARY KEY,
  name TEXT    NOT NULL,
  up   TEXT    NOT NULL,
  down TEXT    NOT NULL
);

CREATE TABLE progress (type TEXT, id TEXT, user_ref TEXT, parent TEXT, progress INTEGER, modified INTEGER DEFAULT 0, PRIMARY KEY (type, id, user_ref)) WITHOUT ROWID;



CREATE TABLE uploadkeys (id TEXT PRIMARY KEY, library_ref TEXT NOT NULL, expiry INTEGER, tags INTEGER DEFAULT "0") WITHOUT ROWID;

CREATE TRIGGER inserted_watched AFTER INSERT ON Watched
            BEGIN
             update Watched SET modified = round((julianday('now') - 2440587.5)*86400.0 * 1000) WHERE id = NEW.id;
            END;
CREATE TRIGGER modified_watched AFTER UPDATE ON Watched
            BEGIN
             update Watched SET modified = round((julianday('now') - 2440587.5)*86400.0 * 1000) WHERE id = NEW.id;
            END;


CREATE TRIGGER inserted_progress AFTER INSERT ON progress
            BEGIN
             update progress SET modified = round((julianday('now') - 2440587.5)*86400.0 * 1000) WHERE id = NEW.id;
            END;
CREATE TRIGGER modified_progress AFTER UPDATE ON progress
            BEGIN
             update progress SET modified = round((julianday('now') - 2440587.5)*86400.0 * 1000) WHERE id = NEW.id;
            END;

CREATE TRIGGER inserted_backup_file AFTER INSERT ON Backups_Files
            BEGIN
             update Backups_Files SET added = round((julianday('now') - 2440587.5)*86400.0 * 1000) WHERE id = NEW.id;
            END;
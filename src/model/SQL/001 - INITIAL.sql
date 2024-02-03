CREATE TABLE Backups (
  id   TEXT PRIMARY KEY,
  source TEXT    NOT NULL,
  credentials TEXT,
  library TEXT NOT NULL,
  path TEXT NOT NULL,
  schedule TEXT ,
  filter TEXT,
  last INTEGER, password TEXT, size INTEGER);

CREATE TABLE Backups_Files (
  backup   TEXT NOT NULL,
  library TEXT NOT NULL,
  file TEXT NOT NULL,
  id TEXT,
  path TEXT,
  hash TEXT,
  sourcehash TEXT,
  size INTEGER,
  date INTEFER,
  iv TEXT,
  thumbsize NUMBER,
  infoSize NUMBER,
  error TEXT,
  FOREIGN KEY (backup) REFERENCES Backups(id),
  FOREIGN KEY (library) REFERENCES Libraries(id),
  PRIMARY KEY (backup, library, file)
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
  crypt INTEGER DEFAULT 0
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
  source TEXT    NOT NULL,
  id TEXT    NOT NULL,
  user_ref TEXT    , date INTEGER, extid TEXT, 'modified' INTEGER,
  PRIMARY KEY (source, id, user_ref)
);

CREATE TABLE "migrations" (
  id   INTEGER PRIMARY KEY,
  name TEXT    NOT NULL,
  up   TEXT    NOT NULL,
  down TEXT    NOT NULL
);

CREATE TABLE progress (type TEXT, id TEXT, user_ref TEXT, progress INTEGER, ids TEXT, PRIMARY KEY (type, id, user_ref)) WITHOUT ROWID;




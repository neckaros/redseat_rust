ALTER TABLE Backups ADD COLUMN maxVersions INTEGER NOT NULL DEFAULT 1 CHECK (maxVersions >= 1);
ALTER TABLE Backups ADD COLUMN maxDatabaseVersions INTEGER NOT NULL DEFAULT 3 CHECK (maxDatabaseVersions >= 1);

CREATE TABLE IF NOT EXISTS library_encryption_jobs (
    id TEXT PRIMARY KEY NOT NULL,
    library_id TEXT NOT NULL UNIQUE,
    source_password TEXT,
    target_password TEXT,
    phase TEXT NOT NULL,
    snapshot_complete INTEGER NOT NULL DEFAULT 0,
    total_items INTEGER NOT NULL DEFAULT 0,
    completed_items INTEGER NOT NULL DEFAULT 0,
    last_error TEXT,
    created INTEGER NOT NULL DEFAULT (unixepoch()),
    modified INTEGER NOT NULL DEFAULT (unixepoch()),
    FOREIGN KEY (library_id) REFERENCES Libraries(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS library_encryption_items (
    id TEXT PRIMARY KEY NOT NULL,
    job_id TEXT NOT NULL,
    kind TEXT NOT NULL,
    media_id TEXT,
    source TEXT NOT NULL,
    staged_source TEXT,
    state TEXT NOT NULL DEFAULT 'pending',
    FOREIGN KEY (job_id) REFERENCES library_encryption_jobs(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_library_encryption_items_job_state
    ON library_encryption_items(job_id, state);

CREATE TABLE request_processing (
    id TEXT PRIMARY KEY,
    processing_id TEXT NOT NULL,
    plugin_id TEXT NOT NULL,
    progress INTEGER NOT NULL DEFAULT 0,
    status TEXT NOT NULL DEFAULT 'pending',
    error TEXT,
    eta INTEGER,
    media_ref TEXT,
    original_request TEXT,
    modified INTEGER,
    added INTEGER,
    FOREIGN KEY (media_ref) REFERENCES medias(id) ON DELETE SET NULL
) WITHOUT ROWID;

CREATE INDEX request_processing_media ON request_processing(media_ref);

CREATE TRIGGER inserted_request_processing AFTER INSERT ON request_processing
BEGIN
    UPDATE request_processing
    SET modified = round((julianday('now') - 2440587.5)*86400.0 * 1000),
        added = round((julianday('now') - 2440587.5)*86400.0 * 1000)
    WHERE id = NEW.id;
END;

CREATE TRIGGER modified_request_processing AFTER UPDATE ON request_processing
BEGIN
    UPDATE request_processing
    SET modified = round((julianday('now') - 2440587.5)*86400.0 * 1000)
    WHERE id = NEW.id;
END;

CREATE TABLE IF NOT EXISTS channels (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    tvg_id TEXT,
    logo TEXT,
    group_tag TEXT,
    channel_number INTEGER,
    modified INTEGER,
    added INTEGER
) WITHOUT ROWID;

CREATE TABLE IF NOT EXISTS channel_variants (
    id TEXT PRIMARY KEY,
    channel_ref TEXT NOT NULL,
    quality TEXT,
    stream_url TEXT NOT NULL,
    modified INTEGER,
    added INTEGER,
    FOREIGN KEY (channel_ref) REFERENCES channels(id) ON DELETE CASCADE
) WITHOUT ROWID;

CREATE INDEX IF NOT EXISTS idx_channels_tvg_id ON channels(tvg_id);
CREATE INDEX IF NOT EXISTS idx_channel_variants_channel_ref ON channel_variants(channel_ref);

CREATE TRIGGER IF NOT EXISTS inserted_channel AFTER INSERT ON channels
BEGIN
    UPDATE channels SET
        modified = round((julianday('now') - 2440587.5)*86400.0 * 1000),
        added = round((julianday('now') - 2440587.5)*86400.0 * 1000)
    WHERE id = NEW.id;
END;

CREATE TRIGGER IF NOT EXISTS modified_channel AFTER UPDATE OF name, tvg_id, logo, group_tag, channel_number ON channels
BEGIN
    UPDATE channels SET modified = round((julianday('now') - 2440587.5)*86400.0 * 1000)
    WHERE id = NEW.id;
END;

CREATE TRIGGER IF NOT EXISTS inserted_channel_variant AFTER INSERT ON channel_variants
BEGIN
    UPDATE channel_variants SET
        modified = round((julianday('now') - 2440587.5)*86400.0 * 1000),
        added = round((julianday('now') - 2440587.5)*86400.0 * 1000)
    WHERE id = NEW.id;
END;

CREATE TRIGGER IF NOT EXISTS modified_channel_variant AFTER UPDATE OF quality, stream_url ON channel_variants
BEGIN
    UPDATE channel_variants SET modified = round((julianday('now') - 2440587.5)*86400.0 * 1000)
    WHERE id = NEW.id;
END;

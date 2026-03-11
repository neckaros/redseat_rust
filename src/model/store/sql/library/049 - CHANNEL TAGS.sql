-- Channel tag mapping (many-to-many)
-- confidence = 0: auto-imported from M3U group-title
-- confidence IS NULL: user-assigned
CREATE TABLE IF NOT EXISTS channel_tag_mapping (
    channel_ref TEXT NOT NULL,
    tag_ref TEXT NOT NULL,
    confidence INTEGER,
    PRIMARY KEY (channel_ref, tag_ref)
);

-- Migrate existing group_tag data
INSERT OR IGNORE INTO channel_tag_mapping (channel_ref, tag_ref, confidence)
SELECT id, group_tag, 0 FROM channels WHERE group_tag IS NOT NULL;

-- Triggers for modified timestamp
CREATE TRIGGER IF NOT EXISTS modified_channels_tags_insert AFTER INSERT ON channel_tag_mapping
BEGIN
    UPDATE channels SET modified = round((julianday('now') - 2440587.5)*86400.0 * 1000)
    WHERE id = NEW.channel_ref;
END;

CREATE TRIGGER IF NOT EXISTS modified_channels_tags_delete AFTER DELETE ON channel_tag_mapping
BEGIN
    UPDATE channels SET modified = round((julianday('now') - 2440587.5)*86400.0 * 1000)
    WHERE id = OLD.channel_ref;
END;

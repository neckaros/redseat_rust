ALTER TABLE channels ADD COLUMN posterv INTEGER;

DROP TRIGGER IF EXISTS modified_channel;
CREATE TRIGGER IF NOT EXISTS modified_channel AFTER UPDATE OF name, tvg_id, logo, group_tag, channel_number, posterv ON channels
BEGIN
    UPDATE channels SET modified = round((julianday('now') - 2440587.5)*86400.0 * 1000)
    WHERE id = NEW.id;
END;

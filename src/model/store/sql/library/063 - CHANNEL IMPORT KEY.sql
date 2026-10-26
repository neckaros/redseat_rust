-- The playlist entries of a channel: its country prefix and its name without the
-- quality. Channels imported before have none and are recreated on the next refresh.
ALTER TABLE channels ADD COLUMN import_key TEXT;
CREATE INDEX IF NOT EXISTS idx_channels_import_key ON channels(import_key);

-- The country prefix of the playlist entries, like `FR` for `|FR| TF1 HD`.
ALTER TABLE channels ADD COLUMN country TEXT;

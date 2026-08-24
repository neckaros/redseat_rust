-- Incremental clients filter by modified/date and then deterministically order by
-- the entity key. Composite indexes avoid full scans as libraries grow.
CREATE INDEX IF NOT EXISTS idx_people_sync ON people(modified, id);
CREATE INDEX IF NOT EXISTS idx_tags_sync ON tags(modified, id);
CREATE INDEX IF NOT EXISTS idx_series_sync ON series(modified, id);
CREATE INDEX IF NOT EXISTS idx_movies_sync ON movies(modified, id);
CREATE INDEX IF NOT EXISTS idx_books_sync ON books(modified, id);
CREATE INDEX IF NOT EXISTS idx_episodes_sync ON episodes(modified, serie_ref, season, number);
CREATE INDEX IF NOT EXISTS idx_deleted_sync ON deleted(date, id, type);
CREATE INDEX IF NOT EXISTS idx_channels_sync ON channels(modified, id);

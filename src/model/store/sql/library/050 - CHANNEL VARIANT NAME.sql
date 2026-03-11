ALTER TABLE channel_variants ADD COLUMN name TEXT;
ALTER TABLE channel_variants ADD COLUMN tvg_name TEXT;
DELETE FROM channel_variants WHERE tvg_name IS NULL;

-- Remove broken plugin-specific IDs from media mapping tables.
-- Plugin IDs (containing ':') were stored instead of internal IDs
-- due to nhentai plugin providing add_people/add_tags with plugin identifiers.

-- Delete media_people_mapping entries with plugin IDs (not internal nanoid format)
DELETE FROM media_people_mapping WHERE people_ref LIKE '%:%';

-- Delete media_tag_mapping entries with plugin IDs
DELETE FROM media_tag_mapping WHERE tag_ref LIKE '%:%';

-- Clear book column on medias where it contains a plugin ID instead of internal ID
UPDATE medias SET book = NULL WHERE book LIKE '%:%';

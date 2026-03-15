-- ================================================
-- Migration 051: CASCADE DELETE TRIGGERS
-- Replaces manual cleanup with trigger-based cascades
-- ================================================

-- 1. MEDIA CHILDREN CASCADE
-- When a media is deleted, remove all referencing records
CREATE TRIGGER cascade_delete_media_children AFTER DELETE ON medias
BEGIN
    DELETE FROM media_tag_mapping WHERE media_ref = OLD.id;
    DELETE FROM media_serie_mapping WHERE media_ref = OLD.id;
    DELETE FROM media_people_mapping WHERE media_ref = OLD.id;
    DELETE FROM shares WHERE media_ref = OLD.id;
    DELETE FROM unassigned_faces WHERE media_ref = OLD.id;
    DELETE FROM people_faces WHERE media_ref = OLD.id;
    DELETE FROM ratings WHERE type = 'media' AND ref = OLD.id;
    DELETE FROM media_progress WHERE media_ref = OLD.id;
    UPDATE request_processing SET media_ref = NULL WHERE media_ref = OLD.id;
END;

-- 2. MOVIE → MEDIAS CASCADE DELETE
DROP TRIGGER IF EXISTS modified_modified_movie_delete;
CREATE TRIGGER cascade_delete_movie_medias AFTER DELETE ON movies
BEGIN
    DELETE FROM medias WHERE movie = OLD.id;
END;

-- 3. BOOK CASCADE DELETE
-- When a book is deleted, remove its medias and mappings
DROP TRIGGER IF EXISTS modified_medias_book_delete;
CREATE TRIGGER cascade_delete_book_children AFTER DELETE ON books
BEGIN
    DELETE FROM medias WHERE book = OLD.id;
    DELETE FROM book_tag_mapping WHERE book_ref = OLD.id;
    DELETE FROM book_people_mapping WHERE book_ref = OLD.id;
END;

-- 4. SERIES CASCADE DELETE
-- When a series is deleted, remove episodes, mappings, and detach books
CREATE TRIGGER cascade_delete_serie_children AFTER DELETE ON series
BEGIN
    DELETE FROM episodes WHERE serie_ref = OLD.id;
    DELETE FROM media_serie_mapping WHERE serie_ref = OLD.id;
    UPDATE books SET serie_ref = NULL, chapter = NULL WHERE serie_ref = OLD.id;
END;

-- 5. EPISODE CASCADE DELETE
-- When an episode is deleted, remove its media mappings
CREATE TRIGGER cascade_delete_episode_children AFTER DELETE ON episodes
BEGIN
    DELETE FROM media_serie_mapping
        WHERE serie_ref = OLD.serie_ref AND season = OLD.season AND episode = OLD.number;
END;

-- 6. TAG CASCADE DELETE (path-based to catch all descendants without recursive triggers)
CREATE TRIGGER cascade_delete_tag_children AFTER DELETE ON tags
BEGIN
    -- Clean up mappings for all descendant tags first
    DELETE FROM media_tag_mapping WHERE tag_ref IN (SELECT id FROM tags WHERE path LIKE OLD.path || OLD.name || '/%');
    DELETE FROM book_tag_mapping WHERE tag_ref IN (SELECT id FROM tags WHERE path LIKE OLD.path || OLD.name || '/%');
    DELETE FROM channel_tag_mapping WHERE tag_ref IN (SELECT id FROM tags WHERE path LIKE OLD.path || OLD.name || '/%');
    -- Delete all descendant tags
    DELETE FROM tags WHERE path LIKE OLD.path || OLD.name || '/%';
    -- Clean up mappings for this tag itself
    DELETE FROM media_tag_mapping WHERE tag_ref = OLD.id;
    DELETE FROM book_tag_mapping WHERE tag_ref = OLD.id;
    DELETE FROM channel_tag_mapping WHERE tag_ref = OLD.id;
END;

-- 7. PEOPLE CASCADE DELETE
CREATE TRIGGER cascade_delete_people_children AFTER DELETE ON people
BEGIN
    DELETE FROM media_people_mapping WHERE people_ref = OLD.id;
    DELETE FROM book_people_mapping WHERE people_ref = OLD.id;
    DELETE FROM people_faces WHERE people_ref = OLD.id;
END;

-- 8. CHANNEL CASCADE DELETE
CREATE TRIGGER cascade_delete_channel_children AFTER DELETE ON channels
BEGIN
    DELETE FROM channel_variants WHERE channel_ref = OLD.id;
    DELETE FROM channel_tag_mapping WHERE channel_ref = OLD.id;
END;

-- 9. PEOPLE FACES → media_people_mapping CASCADE
CREATE TRIGGER cascade_delete_face_refs AFTER DELETE ON people_faces
BEGIN
    UPDATE media_people_mapping SET people_face_ref = NULL WHERE people_face_ref = OLD.id;
END;

-- 10. CLEAN UP ALL EXISTING ORPHANED RECORDS
-- Medias pointing to non-existent parents
DELETE FROM medias WHERE movie IS NOT NULL AND movie NOT IN (SELECT id FROM movies);
DELETE FROM medias WHERE book IS NOT NULL AND book NOT IN (SELECT id FROM books);
-- Orphaned episodes
DELETE FROM episodes WHERE serie_ref NOT IN (SELECT id FROM series);
-- Detach books from non-existent series
UPDATE books SET serie_ref = NULL, chapter = NULL WHERE serie_ref IS NOT NULL AND serie_ref NOT IN (SELECT id FROM series);
-- Orphaned media mappings
DELETE FROM media_tag_mapping WHERE media_ref NOT IN (SELECT id FROM medias);
DELETE FROM media_tag_mapping WHERE tag_ref NOT IN (SELECT id FROM tags);
DELETE FROM media_serie_mapping WHERE media_ref NOT IN (SELECT id FROM medias);
DELETE FROM media_serie_mapping WHERE serie_ref NOT IN (SELECT id FROM series);
DELETE FROM media_people_mapping WHERE media_ref NOT IN (SELECT id FROM medias);
DELETE FROM media_people_mapping WHERE people_ref NOT IN (SELECT id FROM people);
-- Orphaned book mappings
DELETE FROM book_tag_mapping WHERE book_ref NOT IN (SELECT id FROM books);
DELETE FROM book_tag_mapping WHERE tag_ref NOT IN (SELECT id FROM tags);
DELETE FROM book_people_mapping WHERE book_ref NOT IN (SELECT id FROM books);
DELETE FROM book_people_mapping WHERE people_ref NOT IN (SELECT id FROM people);
-- Orphaned channel mappings
DELETE FROM channel_tag_mapping WHERE channel_ref NOT IN (SELECT id FROM channels);
DELETE FROM channel_tag_mapping WHERE tag_ref NOT IN (SELECT id FROM tags);
-- Orphaned channel variants
DELETE FROM channel_variants WHERE channel_ref NOT IN (SELECT id FROM channels);
-- Orphaned face data
DELETE FROM people_faces WHERE media_ref IS NOT NULL AND media_ref NOT IN (SELECT id FROM medias);
DELETE FROM people_faces WHERE people_ref NOT IN (SELECT id FROM people);
DELETE FROM unassigned_faces WHERE media_ref NOT IN (SELECT id FROM medias);
-- Orphaned shares, ratings, progress
DELETE FROM shares WHERE media_ref NOT IN (SELECT id FROM medias);
DELETE FROM ratings WHERE type = 'media' AND ref NOT IN (SELECT id FROM medias);
DELETE FROM media_progress WHERE media_ref NOT IN (SELECT id FROM medias);
-- Orphaned request_processing
UPDATE request_processing SET media_ref = NULL WHERE media_ref IS NOT NULL AND media_ref NOT IN (SELECT id FROM medias);
-- Orphaned tag parent references
DELETE FROM tags WHERE parent IS NOT NULL AND parent NOT IN (SELECT id FROM tags);
-- Orphaned people_face_ref in media_people_mapping
UPDATE media_people_mapping SET people_face_ref = NULL WHERE people_face_ref IS NOT NULL AND people_face_ref NOT IN (SELECT id FROM people_faces);

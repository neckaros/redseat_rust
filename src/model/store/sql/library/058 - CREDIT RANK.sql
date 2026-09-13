ALTER TABLE movie_people_mapping ADD COLUMN rank INTEGER CHECK (rank IS NULL OR (typeof(rank) = 'integer' AND rank BETWEEN 0 AND 4294967295));
CREATE TRIGGER modified_movie_people_mapping_rank AFTER UPDATE OF rank ON movie_people_mapping
WHEN NEW.rank IS NOT OLD.rank
BEGIN
 UPDATE movies SET modified = max(coalesce(modified, 0) + 1,
 round((julianday('now') - 2440587.5)*86400.0 * 1000)) WHERE id = NEW.movie_ref;
END;

ALTER TABLE serie_people_mapping ADD COLUMN rank INTEGER CHECK (rank IS NULL OR (typeof(rank) = 'integer' AND rank BETWEEN 0 AND 4294967295));
CREATE TRIGGER modified_serie_people_mapping_rank AFTER UPDATE OF rank ON serie_people_mapping
WHEN NEW.rank IS NOT OLD.rank
BEGIN
 UPDATE series SET modified = max(coalesce(modified, 0) + 1,
 round((julianday('now') - 2440587.5)*86400.0 * 1000)) WHERE id = NEW.serie_ref;
END;

ALTER TABLE book_people_mapping ADD COLUMN rank INTEGER CHECK (rank IS NULL OR (typeof(rank) = 'integer' AND rank BETWEEN 0 AND 4294967295));
CREATE TRIGGER modified_book_people_mapping_rank AFTER UPDATE OF rank ON book_people_mapping
WHEN NEW.rank IS NOT OLD.rank
BEGIN
 UPDATE books SET modified = max(coalesce(modified, 0) + 1,
 round((julianday('now') - 2440587.5)*86400.0 * 1000)) WHERE id = NEW.book_ref;
END;


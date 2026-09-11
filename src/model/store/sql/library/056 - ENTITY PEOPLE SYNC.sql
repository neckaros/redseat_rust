-- Keep metadata and relationship changes visible to strict modified > after sync.
DROP TRIGGER modified_movie;
CREATE TRIGGER modified_movie AFTER UPDATE ON movies
WHEN NEW.modified <= OLD.modified OR NEW.modified IS NULL
BEGIN
    UPDATE movies SET modified = max(coalesce(OLD.modified, 0) + 1,
        round((julianday('now') - 2440587.5)*86400.0 * 1000)) WHERE id = NEW.id;
END;

DROP TRIGGER modified_serie;
CREATE TRIGGER modified_serie AFTER UPDATE ON series
WHEN NEW.modified <= OLD.modified OR NEW.modified IS NULL
BEGIN
    UPDATE series SET modified = max(coalesce(OLD.modified, 0) + 1,
        round((julianday('now') - 2440587.5)*86400.0 * 1000)) WHERE id = NEW.id;
END;

DROP TRIGGER modified_movies_people_insert;
CREATE TRIGGER modified_movies_people_insert AFTER INSERT ON movie_people_mapping
BEGIN
    UPDATE movies SET modified = max(coalesce(modified, 0) + 1,
        round((julianday('now') - 2440587.5)*86400.0 * 1000)) WHERE id = NEW.movie_ref;
END;

DROP TRIGGER modified_movies_people_delete;
CREATE TRIGGER modified_movies_people_delete AFTER DELETE ON movie_people_mapping
BEGIN
    UPDATE movies SET modified = max(coalesce(modified, 0) + 1,
        round((julianday('now') - 2440587.5)*86400.0 * 1000)) WHERE id = OLD.movie_ref;
END;

DROP TRIGGER modified_series_people_insert;
CREATE TRIGGER modified_series_people_insert AFTER INSERT ON serie_people_mapping
BEGIN
    UPDATE series SET modified = max(coalesce(modified, 0) + 1,
        round((julianday('now') - 2440587.5)*86400.0 * 1000)) WHERE id = NEW.serie_ref;
END;

DROP TRIGGER modified_series_people_delete;
CREATE TRIGGER modified_series_people_delete AFTER DELETE ON serie_people_mapping
BEGIN
    UPDATE series SET modified = max(coalesce(modified, 0) + 1,
        round((julianday('now') - 2440587.5)*86400.0 * 1000)) WHERE id = OLD.serie_ref;
END;


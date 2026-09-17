-- File cleanup and media deletion events are coordinated by ModelController.
-- Keeping media detaches it from the deleted movie.
DROP TRIGGER IF EXISTS cascade_delete_movie_medias;
CREATE TRIGGER cascade_delete_movie_medias AFTER DELETE ON movies
BEGIN
    UPDATE medias SET movie = NULL WHERE movie = OLD.id;
END;

-- Keep every media update visible to strict modified > cursor synchronization,
-- including movie detachment performed by the trigger above.
DROP TRIGGER IF EXISTS modified_medias;
CREATE TRIGGER modified_medias
AFTER UPDATE OF source, name, size, type, mimetype, width, height, duration, thumb,
    params, md5, iv, thumbsize, long, lat, model, origin, thumbv, pages, acodecs,
    achan, vcodecs, fps, bitrate, colorSpance, focal, iso, sspeed, orientation,
    phash, thumbhash, movie, description ON medias
WHEN NEW.modified <= OLD.modified OR NEW.modified IS NULL
BEGIN
    UPDATE medias
    SET modified = max(coalesce(OLD.modified, 0) + 1,
        round((julianday('now') - 2440587.5) * 86400.0 * 1000))
    WHERE id = NEW.id;
END;

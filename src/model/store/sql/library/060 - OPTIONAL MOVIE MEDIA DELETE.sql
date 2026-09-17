-- File cleanup and media deletion events are coordinated by ModelController.
-- Keeping media detaches it from the deleted movie.
DROP TRIGGER IF EXISTS cascade_delete_movie_medias;
CREATE TRIGGER cascade_delete_movie_medias AFTER DELETE ON movies
BEGIN
    UPDATE medias SET movie = NULL WHERE movie = OLD.id;
END;

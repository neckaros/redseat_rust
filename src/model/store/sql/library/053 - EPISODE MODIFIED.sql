DROP TRIGGER IF EXISTS modified_episode;

CREATE TRIGGER modified_episode
AFTER UPDATE OF abs, name, overview, airdate, duration, alt, params, imdb, slug,
                tmdb, trakt, tvdb, otherids, imdb_rating, imdb_votes,
                trakt_rating, trakt_votes
ON episodes
BEGIN
    UPDATE episodes
    SET modified = round((julianday('now') - 2440587.5) * 86400.0 * 1000)
    WHERE serie_ref = NEW.serie_ref
      AND season = NEW.season
      AND number = NEW.number;

    UPDATE series
    SET modified = round((julianday('now') - 2440587.5) * 86400.0 * 1000)
    WHERE id = NEW.serie_ref;
END;

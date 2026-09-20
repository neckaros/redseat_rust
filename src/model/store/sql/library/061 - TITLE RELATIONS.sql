CREATE TABLE movie_tag_mapping (
    movie_ref TEXT NOT NULL,
    tag_ref TEXT NOT NULL,
    confidence INTEGER,
    PRIMARY KEY (movie_ref, tag_ref)
);

CREATE TABLE movie_serie_mapping (
    movie_ref TEXT NOT NULL,
    serie_ref TEXT NOT NULL,
    season INTEGER,
    episode INTEGER,
    episode_to INTEGER,
    PRIMARY KEY (movie_ref, serie_ref, season, episode)
);
CREATE UNIQUE INDEX movie_serie_mapping_unique
ON movie_serie_mapping (movie_ref, serie_ref, ifnull(season, -1), ifnull(episode, -1));

CREATE TABLE serie_tag_mapping (
    serie_ref TEXT NOT NULL,
    tag_ref TEXT NOT NULL,
    confidence INTEGER,
    PRIMARY KEY (serie_ref, tag_ref)
);

CREATE TRIGGER modified_movies_tags_insert AFTER INSERT ON movie_tag_mapping BEGIN
    UPDATE movies SET modified = max(coalesce(modified, 0) + 1,
        round((julianday('now') - 2440587.5) * 86400.0 * 1000)) WHERE id = NEW.movie_ref;
END;
CREATE TRIGGER modified_movies_tags_delete AFTER DELETE ON movie_tag_mapping BEGIN
    UPDATE movies SET modified = max(coalesce(modified, 0) + 1,
        round((julianday('now') - 2440587.5) * 86400.0 * 1000)) WHERE id = OLD.movie_ref;
END;
CREATE TRIGGER modified_movies_series_insert AFTER INSERT ON movie_serie_mapping BEGIN
    UPDATE movies SET modified = max(coalesce(modified, 0) + 1,
        round((julianday('now') - 2440587.5) * 86400.0 * 1000)) WHERE id = NEW.movie_ref;
END;
CREATE TRIGGER modified_movies_series_delete AFTER DELETE ON movie_serie_mapping BEGIN
    UPDATE movies SET modified = max(coalesce(modified, 0) + 1,
        round((julianday('now') - 2440587.5) * 86400.0 * 1000)) WHERE id = OLD.movie_ref;
END;
CREATE TRIGGER modified_series_tags_insert AFTER INSERT ON serie_tag_mapping BEGIN
    UPDATE series SET modified = max(coalesce(modified, 0) + 1,
        round((julianday('now') - 2440587.5) * 86400.0 * 1000)) WHERE id = NEW.serie_ref;
END;
CREATE TRIGGER modified_series_tags_delete AFTER DELETE ON serie_tag_mapping BEGIN
    UPDATE series SET modified = max(coalesce(modified, 0) + 1,
        round((julianday('now') - 2440587.5) * 86400.0 * 1000)) WHERE id = OLD.serie_ref;
END;

CREATE TRIGGER cleanup_movie_relations AFTER DELETE ON movies BEGIN
    DELETE FROM movie_tag_mapping WHERE movie_ref = OLD.id;
    DELETE FROM movie_serie_mapping WHERE movie_ref = OLD.id;
END;
CREATE TRIGGER cleanup_serie_title_relations AFTER DELETE ON series BEGIN
    DELETE FROM serie_tag_mapping WHERE serie_ref = OLD.id;
    DELETE FROM movie_serie_mapping WHERE serie_ref = OLD.id;
END;
CREATE TRIGGER cleanup_tag_title_relations AFTER DELETE ON tags BEGIN
    DELETE FROM movie_tag_mapping WHERE tag_ref = OLD.id;
    DELETE FROM serie_tag_mapping WHERE tag_ref = OLD.id;
END;

DELETE FROM movie_tag_mapping WHERE movie_ref NOT IN (SELECT id FROM movies) OR tag_ref NOT IN (SELECT id FROM tags);
DELETE FROM movie_serie_mapping WHERE movie_ref NOT IN (SELECT id FROM movies) OR serie_ref NOT IN (SELECT id FROM series);
DELETE FROM serie_tag_mapping WHERE serie_ref NOT IN (SELECT id FROM series) OR tag_ref NOT IN (SELECT id FROM tags);

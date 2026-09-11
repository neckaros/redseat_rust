CREATE TABLE movie_people_mapping (
    movie_ref TEXT NOT NULL REFERENCES movies(id),
    people_ref TEXT NOT NULL REFERENCES people(id),
    PRIMARY KEY (movie_ref, people_ref)
);
CREATE INDEX movie_people_person ON movie_people_mapping(people_ref);
CREATE TABLE serie_people_mapping (
    serie_ref TEXT NOT NULL REFERENCES series(id),
    people_ref TEXT NOT NULL REFERENCES people(id),
    PRIMARY KEY (serie_ref, people_ref)
);
CREATE INDEX serie_people_person ON serie_people_mapping(people_ref);

CREATE TRIGGER modified_movies_people_insert AFTER INSERT ON movie_people_mapping BEGIN
    UPDATE movies SET modified = round((julianday('now') - 2440587.5)*86400.0 * 1000) WHERE id = NEW.movie_ref;
END;
CREATE TRIGGER modified_movies_people_delete AFTER DELETE ON movie_people_mapping BEGIN
    UPDATE movies SET modified = round((julianday('now') - 2440587.5)*86400.0 * 1000) WHERE id = OLD.movie_ref;
END;
CREATE TRIGGER modified_series_people_insert AFTER INSERT ON serie_people_mapping BEGIN
    UPDATE series SET modified = round((julianday('now') - 2440587.5)*86400.0 * 1000) WHERE id = NEW.serie_ref;
END;
CREATE TRIGGER modified_series_people_delete AFTER DELETE ON serie_people_mapping BEGIN
    UPDATE series SET modified = round((julianday('now') - 2440587.5)*86400.0 * 1000) WHERE id = OLD.serie_ref;
END;
CREATE TRIGGER delete_movie_people BEFORE DELETE ON movies BEGIN
    DELETE FROM movie_people_mapping WHERE movie_ref = OLD.id;
END;
CREATE TRIGGER delete_serie_people BEFORE DELETE ON series BEGIN
    DELETE FROM serie_people_mapping WHERE serie_ref = OLD.id;
END;
CREATE TRIGGER delete_person_entities BEFORE DELETE ON people BEGIN
    DELETE FROM movie_people_mapping WHERE people_ref = OLD.id;
    DELETE FROM serie_people_mapping WHERE people_ref = OLD.id;
END;

CREATE TRIGGER inserted_episode AFTER INSERT ON episodes
            BEGIN
             update episodes SET modified = round((julianday('now') - 2440587.5)*86400.0 * 1000), added = round((julianday('now') - 2440587.5)*86400.0 * 1000) WHERE serie_ref = NEW.serie_ref and season = NEW.season and number = NEW.number;
            END;


CREATE TRIGGER inserted_serie AFTER INSERT ON series
            BEGIN
             update series SET modified = round((julianday('now') - 2440587.5)*86400.0 * 1000), added = round((julianday('now') - 2440587.5)*86400.0 * 1000) WHERE id = NEW.id;
            END;

CREATE TRIGGER inserted_tags AFTER INSERT ON tags
            BEGIN
             update tags SET modified = round((julianday('now') - 2440587.5)*86400.0 * 1000), added = round((julianday('now') - 2440587.5)*86400.0 * 1000) WHERE id = NEW.id;
            END;

CREATE TRIGGER inserted_people AFTER INSERT ON people
            BEGIN
             update people SET modified = round((julianday('now') - 2440587.5)*86400.0 * 1000), added = round((julianday('now') - 2440587.5)*86400.0 * 1000) WHERE id = NEW.id;
            END;

CREATE TRIGGER inserted_medias AFTER INSERT ON medias
            BEGIN
             update medias SET modified = round((julianday('now') - 2440587.5)*86400.0 * 1000), added = round((julianday('now') - 2440587.5)*86400.0 * 1000), created = ifnull(created, round((julianday('now') - 2440587.5)*86400.0 * 1000)) WHERE id = NEW.id;
            END;

CREATE TRIGGER inserted_movie AFTER INSERT ON movies
            BEGIN
             update movies SET modified = round((julianday('now') - 2440587.5)*86400.0 * 1000), added = round((julianday('now') - 2440587.5)*86400.0 * 1000) WHERE id = NEW.id;
            END;

CREATE TRIGGER modified_movie AFTER UPDATE ON movies
            BEGIN
             update movies SET modified = round((julianday('now') - 2440587.5)*86400.0 * 1000) WHERE id = NEW.id;
            END;

CREATE TRIGGER inserted_deleted AFTER INSERT ON deleted
            BEGIN
             update deleted SET date = round((julianday('now') - 2440587.5)*86400.0 * 1000) WHERE id = NEW.id and type = NEW.type;
            END;
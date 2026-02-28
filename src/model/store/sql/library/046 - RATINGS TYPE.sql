CREATE TABLE ratings_new (
    type TEXT NOT NULL DEFAULT 'media',
    ref TEXT NOT NULL,
    user_ref TEXT NOT NULL,
    rating REAL,
    modified INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (type, ref, user_ref)
);

INSERT INTO ratings_new (type, ref, user_ref, rating, modified)
    SELECT 'media', media_ref, user_ref, rating, modified FROM ratings;

DROP TRIGGER IF EXISTS inserted_rating;
DROP TRIGGER IF EXISTS updated_ratings;
DROP TABLE ratings;
ALTER TABLE ratings_new RENAME TO ratings;

CREATE TRIGGER inserted_rating AFTER INSERT ON ratings
    BEGIN
        UPDATE ratings SET modified = round((julianday('now') - 2440587.5)*86400.0 * 1000)
        WHERE type = NEW.type AND ref = NEW.ref AND user_ref = NEW.user_ref;
    END;

CREATE TRIGGER updated_ratings AFTER UPDATE ON ratings
    BEGIN
        UPDATE ratings SET modified = round((julianday('now') - 2440587.5)*86400.0 * 1000)
        WHERE type = NEW.type AND ref = NEW.ref AND user_ref = NEW.user_ref;
    END;

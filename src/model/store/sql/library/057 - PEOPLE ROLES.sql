ALTER TABLE movie_people_mapping ADD COLUMN characters TEXT CHECK (characters IS NULL OR (json_valid(characters) AND json_type(characters) = 'array'));
ALTER TABLE serie_people_mapping ADD COLUMN characters TEXT CHECK (characters IS NULL OR (json_valid(characters) AND json_type(characters) = 'array'));
ALTER TABLE book_people_mapping ADD COLUMN characters TEXT CHECK (characters IS NULL OR (json_valid(characters) AND json_type(characters) = 'array'));
ALTER TABLE movie_people_mapping ADD COLUMN roles TEXT CHECK (roles IS NULL OR (json_valid(roles) AND json_type(roles) = 'array'));
ALTER TABLE serie_people_mapping ADD COLUMN roles TEXT CHECK (roles IS NULL OR (json_valid(roles) AND json_type(roles) = 'array'));
ALTER TABLE book_people_mapping ADD COLUMN roles TEXT CHECK (roles IS NULL OR (json_valid(roles) AND json_type(roles) = 'array'));

CREATE TRIGGER modified_movie_people_mapping_roles AFTER UPDATE OF roles, characters ON movie_people_mapping
WHEN NEW.roles IS NOT OLD.roles OR NEW.characters IS NOT OLD.characters
BEGIN
 UPDATE movies SET modified = max(coalesce(modified, 0) + 1,
 round((julianday('now') - 2440587.5)*86400.0 * 1000)) WHERE id = NEW.movie_ref;
END;

CREATE TRIGGER modified_serie_people_mapping_roles AFTER UPDATE OF roles, characters ON serie_people_mapping
WHEN NEW.roles IS NOT OLD.roles OR NEW.characters IS NOT OLD.characters
BEGIN
 UPDATE series SET modified = max(coalesce(modified, 0) + 1,
 round((julianday('now') - 2440587.5)*86400.0 * 1000)) WHERE id = NEW.serie_ref;
END;

CREATE TRIGGER modified_book_people_mapping_roles AFTER UPDATE OF roles, characters, confidence ON book_people_mapping
WHEN NEW.roles IS NOT OLD.roles OR NEW.characters IS NOT OLD.characters OR NEW.confidence IS NOT OLD.confidence
BEGIN
 UPDATE books SET modified = max(coalesce(modified, 0) + 1,
 round((julianday('now') - 2440587.5)*86400.0 * 1000)) WHERE id = NEW.book_ref;
END;

DROP TRIGGER modified_book;
CREATE TRIGGER modified_book AFTER UPDATE ON books
WHEN NEW.modified <= OLD.modified OR NEW.modified IS NULL
BEGIN
 UPDATE books SET modified = max(coalesce(OLD.modified, 0) + 1,
 round((julianday('now') - 2440587.5)*86400.0 * 1000)) WHERE id = NEW.id;
END;

CREATE INDEX IF NOT EXISTS book_people_person ON book_people_mapping(people_ref);

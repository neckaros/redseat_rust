ALTER TABLE medias ADD COLUMN book TEXT;

ALTER TABLE series ADD COLUMN openlibrary_work_id TEXT;
ALTER TABLE series ADD COLUMN anilist_manga_id INTEGER;
ALTER TABLE series ADD COLUMN mangadex_manga_uuid TEXT;
ALTER TABLE series ADD COLUMN myanimelist_manga_id INTEGER;

CREATE TABLE books (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    type TEXT,
    serie_ref TEXT,
    volume REAL,
    chapter REAL,
    year INTEGER,
    airdate INTEGER,
    overview TEXT,
    pages INTEGER,
    params TEXT,
    lang TEXT,
    original TEXT,
    isbn13 TEXT,
    openlibrary_edition_id TEXT,
    openlibrary_work_id TEXT,
    google_books_volume_id TEXT,
    asin TEXT,
    modified INTEGER,
    added INTEGER,
    CHECK (chapter IS NULL OR serie_ref IS NOT NULL)
) WITHOUT ROWID;

CREATE INDEX idx_books_serie_ref ON books(serie_ref);
CREATE INDEX idx_books_isbn13 ON books(isbn13);
CREATE INDEX idx_books_openlibrary_edition_id ON books(openlibrary_edition_id);
CREATE INDEX idx_books_openlibrary_work_id ON books(openlibrary_work_id);
CREATE INDEX idx_books_google_books_volume_id ON books(google_books_volume_id);
CREATE INDEX idx_books_asin ON books(asin);
CREATE INDEX idx_books_serie_volume_chapter ON books(serie_ref, volume, chapter);

CREATE TRIGGER inserted_book AFTER INSERT ON books
BEGIN
    UPDATE books
    SET modified = round((julianday('now') - 2440587.5) * 86400.0 * 1000),
        added = round((julianday('now') - 2440587.5) * 86400.0 * 1000)
    WHERE id = NEW.id;
END;

CREATE TRIGGER modified_book AFTER UPDATE ON books
BEGIN
    UPDATE books
    SET modified = round((julianday('now') - 2440587.5) * 86400.0 * 1000)
    WHERE id = NEW.id;
END;

CREATE TRIGGER modified_medias_book_update AFTER UPDATE OF book ON medias
BEGIN
    UPDATE medias
    SET modified = round((julianday('now') - 2440587.5) * 86400.0 * 1000)
    WHERE id = NEW.id;
END;

CREATE TRIGGER modified_medias_book_delete AFTER DELETE ON books
BEGIN
    UPDATE medias
    SET book = NULL
    WHERE book = OLD.id;
END;

ALTER TABLE medias DROP COLUMN isbn13;
ALTER TABLE medias DROP COLUMN openlibrary_edition_id;
ALTER TABLE medias DROP COLUMN openlibrary_work_id;
ALTER TABLE medias DROP COLUMN google_books_volume_id;
ALTER TABLE medias DROP COLUMN anilist_manga_id;
ALTER TABLE medias DROP COLUMN mangadex_manga_uuid;
ALTER TABLE medias DROP COLUMN myanimelist_manga_id;
ALTER TABLE medias DROP COLUMN asin;

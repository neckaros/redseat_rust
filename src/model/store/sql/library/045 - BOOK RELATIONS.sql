CREATE TABLE book_tag_mapping (
    book_ref  TEXT NOT NULL,
    tag_ref   TEXT NOT NULL,
    confidence INTEGER,
    PRIMARY KEY (book_ref, tag_ref)
);

CREATE TABLE book_people_mapping (
    book_ref    TEXT NOT NULL,
    people_ref  TEXT NOT NULL,
    confidence  INTEGER,
    PRIMARY KEY (book_ref, people_ref)
);

CREATE TRIGGER modified_books_tags_insert AFTER INSERT ON book_tag_mapping
BEGIN
    UPDATE books SET modified = round((julianday('now') - 2440587.5)*86400.0 * 1000)
    WHERE id = NEW.book_ref;
END;

CREATE TRIGGER modified_books_tags_delete AFTER DELETE ON book_tag_mapping
BEGIN
    UPDATE books SET modified = round((julianday('now') - 2440587.5)*86400.0 * 1000)
    WHERE id = OLD.book_ref;
END;

CREATE TRIGGER modified_books_people_insert AFTER INSERT ON book_people_mapping
BEGIN
    UPDATE books SET modified = round((julianday('now') - 2440587.5)*86400.0 * 1000)
    WHERE id = NEW.book_ref;
END;

CREATE TRIGGER modified_books_people_delete AFTER DELETE ON book_people_mapping
BEGIN
    UPDATE books SET modified = round((julianday('now') - 2440587.5)*86400.0 * 1000)
    WHERE id = OLD.book_ref;
END;

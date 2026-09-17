-- Media deletion is coordinated by ModelController so files and events are handled.
-- Keeping media detaches it from the deleted book.
DROP TRIGGER IF EXISTS cascade_delete_book_children;
CREATE TRIGGER cascade_delete_book_children AFTER DELETE ON books
BEGIN
    UPDATE medias SET book = NULL WHERE book = OLD.id;
    DELETE FROM book_tag_mapping WHERE book_ref = OLD.id;
    DELETE FROM book_people_mapping WHERE book_ref = OLD.id;
END;

DROP TRIGGER IF EXISTS modified_medias_book_update;
CREATE TRIGGER modified_medias_book_update AFTER UPDATE OF book ON medias
WHEN NEW.modified <= OLD.modified OR NEW.modified IS NULL
BEGIN
    UPDATE medias
    SET modified = max(coalesce(OLD.modified, 0) + 1,
        round((julianday('now') - 2440587.5) * 86400.0 * 1000))
    WHERE id = NEW.id;
END;

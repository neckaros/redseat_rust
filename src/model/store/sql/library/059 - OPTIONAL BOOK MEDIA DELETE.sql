-- Media deletion is coordinated by ModelController so files and events are handled.
-- Keeping media detaches it from the deleted book.
DROP TRIGGER IF EXISTS cascade_delete_book_children;
CREATE TRIGGER cascade_delete_book_children AFTER DELETE ON books
BEGIN
    UPDATE medias SET book = NULL WHERE book = OLD.id;
    DELETE FROM book_tag_mapping WHERE book_ref = OLD.id;
    DELETE FROM book_people_mapping WHERE book_ref = OLD.id;
END;

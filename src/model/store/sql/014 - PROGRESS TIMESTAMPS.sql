-- Progress timestamps are per user, and only change when the position does:
-- the old triggers stamped every user's row for the same id, on any update
-- (including a history id rewrite). Updates that set modified keep it.
DROP TRIGGER IF EXISTS inserted_progress;
DROP TRIGGER IF EXISTS modified_progress;
CREATE TRIGGER inserted_progress AFTER INSERT ON progress
            BEGIN
             update progress SET modified = round((julianday('now') - 2440587.5)*86400.0 * 1000)
             WHERE type = NEW.type AND id = NEW.id AND user_ref = NEW.user_ref;
            END;
CREATE TRIGGER modified_progress AFTER UPDATE OF progress ON progress
            WHEN NEW.progress IS NOT OLD.progress AND NEW.modified IS OLD.modified
            BEGIN
             update progress SET modified = round((julianday('now') - 2440587.5)*86400.0 * 1000)
             WHERE type = NEW.type AND id = NEW.id AND user_ref = NEW.user_ref;
            END;

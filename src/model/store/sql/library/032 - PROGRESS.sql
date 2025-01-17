ALTER TABLE ratings ADD COLUMN modified INTEGER NOT NULL DEFAULT 0;
UPDATE ratings set modified = round((julianday('now') - 2440587.5)*86400.0 * 1000);

CREATE TRIGGER inserted_rating AFTER INSERT ON ratings
            BEGIN
             update ratings SET modified = round((julianday('now') - 2440587.5)*86400.0 * 1000) WHERE media_ref = NEW.media_ref and user_ref = NEW.user_ref;
            END;
       
CREATE TRIGGER updated_ratings AFTER UPDATE ON ratings
            BEGIN
             update ratings SET modified = round((julianday('now') - 2440587.5)*86400.0 * 1000) WHERE media_ref = NEW.media_ref and user_ref = NEW.user_ref;
            END;

CREATE TABLE media_progress (media_ref TEXT, user_ref TEXT, progress INTEGER, modified INTEGER NOT NULL DEFAULT 0, PRIMARY KEY (media_ref, user_ref));

CREATE TRIGGER inserted_media_progress AFTER INSERT ON media_progress
            BEGIN
             update media_progress SET modified = round((julianday('now') - 2440587.5)*86400.0 * 1000) WHERE media_ref = NEW.media_ref and user_ref = NEW.user_ref;
            END;
       
CREATE TRIGGER updated_media_progress AFTER UPDATE ON media_progress
            BEGIN
             update media_progress SET modified = round((julianday('now') - 2440587.5)*86400.0 * 1000) WHERE media_ref = NEW.media_ref and user_ref = NEW.user_ref;
            END;


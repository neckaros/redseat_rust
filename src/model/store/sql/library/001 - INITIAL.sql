CREATE TABLE deleted (id TEXT PRIMARY KEY, date INTEGER, type TEXT) WITHOUT ROWID;
CREATE TABLE episodes (serie_ref TEXT, name TEXT, season INTEGER, number INTEGER, abs INTEGER, overview TEXT, airdate INTEGER,  duration INTEGER, type TEXT, alt TEXT, params TEXT, imdb TEXT, slug TEXT, tmdb INTEGER, trakt INTEGER, tvdb INTEGER, otherids TEXT, modified INTEGER, added INTEGER, imdb_rating REAL, imdb_votes INTEGER, trakt_rating REAL, trakt_votes INTEGER, PRIMARY KEY (serie_ref, season, number)) WITHOUT ROWID;
CREATE TABLE media_people_mapping (media_ref TEXT, people_ref TEXT, confidence INTEGER, PRIMARY KEY (media_ref, people_ref));
CREATE TABLE media_serie_mapping (media_ref TEXT, serie_ref TEXT, season INTEGER, episode INTEGER, modified INTEGER, added INTEGER, PRIMARY KEY (media_ref, serie_ref, season, episode));
CREATE TABLE media_tag_mapping (media_ref TEXT, tag_ref TEXT, confidence INTEGER, PRIMARY KEY (media_ref, tag_ref));
CREATE TABLE medias (id TEXT PRIMARY KEY, source TEXT, name TEXT, size INTEGER, type TEXT, mimetype TEXT, md5 TEXT, modified INTEGER, created INTEGER, added INTEGER, starred INTEGER, width INTEGER, height INTEGER, duration INTEGER, thumb TEXT, thumbv INTEGER, iv TEXT, thumbsize INTEGER, long REAL, lat REAL, model TEXT, pages INTEGER, params TEXT, origin TEXT, progress INTEGER, movie TEXT, acodecs TEXT, achan TEXT, vcodecs TEXT, fps INTEGER, bitrate INTEGER, colorSpace TEXT, icc TEXT, mp INTEGER, focal INTEGER, iso INTEGER, sspeed TEXT, fnumber REAL, orientation INTEGER, phash TEXT, thumbhash TEXT, description TEXT, lang TEXT, uploader TEXT, uploadkey TEXT, aitag INTEGER DEFAULT "0" NOT NULL CHECK (aitag IN (0, 1)), aiassign INTEGER DEFAULT "0" NOT NULL CHECK (aitag IN (0, 1))) WITHOUT ROWID;
CREATE TABLE "migrations" (
  id   INTEGER PRIMARY KEY,
  name TEXT    NOT NULL,
  up   TEXT    NOT NULL,
  down TEXT    NOT NULL
);
CREATE TABLE movies (id TEXT PRIMARY KEY, name TEXT, year INTEGER, airdate INTEGER, digitalairdate INTEGER, duration INTEGER, overview TEXT, country TEXT,  status TEXT, type TEXT, params TEXT, imdb TEXT, slug TEXT, tmdb INTEGER, trakt INTEGER, otherids TEXT, modified INTEGER, added INTEGER, lang TEXT, original TEXT, imdb_rating REAL, imdb_votes INTEGER, trailer TEXT, trakt_rating REAL, trakt_votes INTEGER) WITHOUT ROWID;
CREATE TABLE people (id TEXT PRIMARY KEY, name TEXT, socials TEXT, type TEXT, alt TEXT, birthday INTEGER, portrait TEXT, params TEXT, modified INTEGER, added INTEGER) WITHOUT ROWID;
CREATE TABLE people_faces (
                id   TEXT PRIMARY KEY,
                people_ref TEXT    NOT NULL,
                factors TEXT    NOT NULL
            );
CREATE TABLE ratings (media_ref TEXT, user_ref TEXT, rating REAL, PRIMARY KEY (media_ref, user_ref));
CREATE TABLE series (id TEXT PRIMARY KEY, name TEXT, year INTEGER, type TEXT, alt TEXT, params TEXT, poster TEXT, imdb TEXT, slug TEXT, tmdb INTEGER, trakt INTEGER, tvdb INTEGER, otherids TEXT, created INTEGER, status TEXT, overview TEXT, lang TEXT, original TEXT, modified INTEGER, added INTEGER, imdb_rating REAL, imdb_votes INTEGER, trailer TEXT, maxCreated INTEGER, trakt_rating REAL, trakt_votes INTEGER) WITHOUT ROWID;
CREATE TABLE shares (media_ref TEXT, platform TEXT, idex TEXT, params TEXT, PRIMARY KEY (media_ref, platform, idex));
CREATE TABLE tags (id TEXT PRIMARY KEY, name TEXT, parent TEXT, type TEXT, alt TEXT, thumb TEXT, params TEXT, modified INTEGER, added INTEGER, generated INTEGER DEFAULT "0" NOT NULL CHECK (generated IN (0, 1))) WITHOUT ROWID;

CREATE TABLE user_rights (
                user   TEXT NOT NULL,
                type TEXT   TEXT NOT NULL,
                id TEXT   TEXT NOT NULL,
                roles TEXT   TEXT NOT NULL,
                PRIMARY KEY (user, type, id)
            );

CREATE INDEX people_faces_person
            on people_faces(people_ref);


CREATE TRIGGER added_media_serie_mapping AFTER INSERT ON media_serie_mapping
            BEGIN
             update media_serie_mapping SET modified = round((julianday('now') - 2440587.5)*86400.0 * 1000) and added = round((julianday('now') - 2440587.5)*86400.0 * 1000) WHERE media_ref = NEW.media_ref and serie_ref = NEW.serie_ref and season = NEW.season and episode = NEW.episode;
            END;
CREATE TRIGGER modified_episode AFTER UPDATE ON episodes
            BEGIN
             update episodes SET modified = round((julianday('now') - 2440587.5)*86400.0 * 1000) WHERE serie_ref = NEW.serie_ref and season = NEW.season and number = NEW.number and abs = NEW.abs;
             update series SET modified = round((julianday('now') - 2440587.5)*86400.0 * 1000) WHERE id = NEW.serie_ref;
            END;
CREATE TRIGGER modified_media_serie_mapping AFTER UPDATE ON media_serie_mapping
            BEGIN
             update media_serie_mapping SET modified = round((julianday('now') - 2440587.5)*86400.0 * 1000) WHERE media_ref = NEW.media_ref and serie_ref = NEW.serie_ref and season = NEW.season and episode = NEW.episode;
            END;
CREATE TRIGGER modified_medias 
            AFTER UPDATE OF source, name, size, type, mimetype, width, height, duration, thumb, params, md5, iv, thumbsize, long, lat, model, origin, thumbv, pages, acodecs, achan, vcodecs, fps, bitrate, colorSpance, focal, iso, sspeed, orientation, phash, thumbhash, movie, description ON medias
            BEGIN
             update medias SET modified = round((julianday('now') - 2440587.5)*86400.0 * 1000) WHERE id = NEW.id;
            END;
CREATE TRIGGER modified_medias__serie_delete AFTER DELETE ON media_serie_mapping
 BEGIN
  update medias SET modified = round((julianday('now') - 2440587.5)*86400.0 * 1000) WHERE id = OLD.media_ref;
 END;
 CREATE TRIGGER modified_medias_people_delete AFTER DELETE ON media_people_mapping
 BEGIN
  update medias SET modified = round((julianday('now') - 2440587.5)*86400.0 * 1000) WHERE id = OLD.media_ref;
 END;
 CREATE TRIGGER modified_medias_peoples_insert AFTER INSERT ON media_people_mapping
 BEGIN
  update medias SET modified = round((julianday('now') - 2440587.5)*86400.0 * 1000) WHERE id = NEW.media_ref;
 END;
 CREATE TRIGGER modified_medias_serie_insert AFTER INSERT ON media_serie_mapping
 BEGIN
  update medias SET modified = round((julianday('now') - 2440587.5)*86400.0 * 1000) WHERE id = NEW.media_ref;
 END;
 CREATE TRIGGER modified_medias_tags_delete AFTER DELETE ON media_tag_mapping
 BEGIN
  update medias SET modified = round((julianday('now') - 2440587.5)*86400.0 * 1000) WHERE id = OLD.media_ref;
 END;
 CREATE TRIGGER modified_medias_tags_insert AFTER INSERT ON media_tag_mapping
 BEGIN
  update medias SET modified = round((julianday('now') - 2440587.5)*86400.0 * 1000) WHERE id = NEW.media_ref;
 END;
 CREATE TRIGGER modified_modified_movie_delete AFTER DELETE ON movies
 BEGIN
  update medias SET movie = NULL WHERE movie = OLD.id;
 END;
 CREATE TRIGGER modified_movie AFTER UPDATE ON movies
 BEGIN
  update movies SET modified = round((julianday('now') - 2440587.5)*86400.0 * 1000) WHERE id = NEW.id;
 END;
 CREATE TRIGGER modified_people AFTER UPDATE ON people
            BEGIN
             update people SET modified = round((julianday('now') - 2440587.5)*86400.0 * 1000) WHERE id = NEW.id;
            END;
CREATE TRIGGER modified_serie AFTER UPDATE ON series
            BEGIN
             update series SET modified = round((julianday('now') - 2440587.5)*86400.0 * 1000) WHERE id = NEW.id;
            END;
CREATE TRIGGER modified_tags AFTER UPDATE ON tags
            BEGIN
             update tags SET modified = round((julianday('now') - 2440587.5)*86400.0 * 1000) WHERE id = NEW.id;
            END;
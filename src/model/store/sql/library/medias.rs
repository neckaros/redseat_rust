use rusqlite::{params, types::{FromSql, FromSqlError, FromSqlResult, ToSqlOutput, ValueRef}, OptionalExtension, Row, ToSql};

use crate::{domain::media::{FileType, Media, MediaForInsert, MediaForUpdate}, model::{medias::{MediaQuery, MediaSource}, store::{from_comma_separated_optional, from_pipe_separated_optional, sql::{OrderBuilder, QueryBuilder, QueryWhereType, SqlOrder}}}};
use super::{Result, SqliteLibraryStore};
use crate::model::Error;

impl FromSql for FileType {
    fn column_result(value: ValueRef) -> FromSqlResult<Self> {
        String::column_result(value).and_then(|as_string| {
            let r = FileType::try_from(&*as_string).map_err(|_| FromSqlError::InvalidType);
            r
        })
    }
}

impl ToSql for FileType {
    fn to_sql(&self) -> rusqlite::Result<rusqlite::types::ToSqlOutput<'_>> {
        Ok(ToSqlOutput::from(self.to_string()))
    }
}

impl SqliteLibraryStore {
    fn row_to_mediasource(row: &Row) -> rusqlite::Result<MediaSource> {
        Ok(MediaSource {
            id: row.get(0)?,
            source: row.get(1)?,
        })
    }

    fn row_to_media(row: &Row) -> rusqlite::Result<Media> {
        Ok(Media {
            id: row.get(0)?,
            source: row.get(1)?,
            name: row.get(2)?,
            description: row.get(3)?,

            kind: row.get(4)?,
            mimetype: row.get(5)?,
            size: row.get(6)?,

            rating: row.get(7)?,
            md5: row.get(8)?,

            params: row.get(9)?,

            width: row.get(10)?,
            height: row.get(11)?,
            phash: row.get(12)?,
            thumbhash: row.get(13)?,
            focal: row.get(14)?,
            iso: row.get(15)?,
            color_space: row.get(16)?,
            sspeed: row.get(17)?,
            orientation: row.get(18)?,

            duration: row.get(19)?,
            acodecs: from_pipe_separated_optional(row.get(20)?),
            achan: from_pipe_separated_optional(row.get(21)?),
            vcodecs: from_pipe_separated_optional(row.get(22)?),
            fps: row.get(23)?,
            bitrate: row.get(24)?,


 

            long: row.get(25)?,
            lat: row.get(26)?,
            model: row.get(27)?,

            pages: row.get(28)?,

            progress: row.get(29)?,
            thumb: row.get(30)?,
            thumbv: row.get(31)?,

            thumbsize: row.get(32)?,
            iv: row.get(33)?,

            
            origin: row.get(34)?,
            movie: row.get(35)?,
            lang: row.get(36)?,
            uploader: row.get(37)?,
            uploadkey: row.get(38)?,


            modified: row.get(39)?,
            added: row.get(40)?,
            created: row.get(41)?,

         
            tags: from_comma_separated_optional(row.get(42)?),
            people: from_pipe_separated_optional(row.get(43)?),
            series: from_comma_separated_optional(row.get(44)?),
            //series: None,
        })
    }

    pub async fn get_medias(&self, query: MediaQuery) -> Result<Vec<Media>> {
        let row = self.connection.call( move |conn| { 
            let mut where_query = QueryBuilder::new();
            where_query.add_where(query.after, QueryWhereType::After("modified".to_string()));
            if query.after.is_some() {
                where_query.add_oder(OrderBuilder::new("modified".to_string(), SqlOrder::ASC))
            }


            let mut query = conn.prepare(&format!("SELECT 
            m.id, m.source, m.name, m.description, m.type, m.mimetype, m.size, avg(ratings.rating) as rating, m.md5, m.params, 
            m.width, m.height, m.phash, m.thumbhash, m.focal, m.iso, m.colorSpace, m.sspeed, m.orientation, m.duration, 
            m.acodecs, m.achan, m.vcodecs, m.fps, m.bitrate, m.long, m.lat, m.model, m.pages, m.progress, 
            m.thumb, m.thumbv, m.thumbsize, m.iv, m.origin, m.movie, m.lang, m.uploader, m.uploadkey, m.modified, 
            m.added, m.created,
            
            GROUP_CONCAT(distinct a.tag_ref || '|' || IFNULL(a.confidence, 101)) tags,
            GROUP_CONCAT(distinct b.people_ref) people,
            GROUP_CONCAT(distinct c.serie_ref || '|' || ifnull(c.season,'') || '|' || ifnull(printf('%04d', c.episode),'') ) series
            
            FROM medias as m
                LEFT JOIN ratings on ratings.media_ref = m.id
                LEFT JOIN media_tag_mapping a on a.media_ref = m.id and (a.confidence != -1 or a.confidence IS NULL)
                LEFT JOIN media_people_mapping b on b.media_ref = m.id
                LEFT JOIN media_serie_mapping c on c.media_ref = m.id
            
             {}{} 
             GROUP BY m.id
             LIMIT 200", where_query.format(), where_query.format_order()))?;
            let rows = query.query_map(
            where_query.values(), Self::row_to_media,
            )?;
            let backups:Vec<Media> = rows.collect::<std::result::Result<Vec<Media>, rusqlite::Error>>()?; 
            Ok(backups)
        }).await?;
        Ok(row)
    }
    pub async fn get_media(&self, media_id: &str) -> Result<Option<Media>> {
        let media_id = media_id.to_string();
        let row = self.connection.call( move |conn| { 
            let mut query = conn.prepare("SELECT 
            m.id, m.source, m.name, m.description, m.type, m.mimetype, m.size, avg(ratings.rating) as rating, m.md5, m.params, 
            m.width, m.height, m.phash, m.thumbhash, m.focal, m.iso, m.colorSpace, m.sspeed, m.orientation, m.duration, 
            m.acodecs, m.achan, m.vcodecs, m.fps, m.bitrate, m.long, m.lat, m.model, m.pages, m.progress, 
            m.thumb, m.thumbv, m.thumbsize, m.iv, m.origin, m.movie, m.lang, m.uploader, m.uploadkey, m.modified, 
            m.added, m.created,
            
            GROUP_CONCAT(distinct a.tag_ref || '|' || IFNULL(a.confidence, 101)) tags,
            GROUP_CONCAT(distinct b.people_ref) people,
            GROUP_CONCAT(distinct c.serie_ref || '|' || ifnull(c.season,'') || '|' || ifnull(printf('%04d', c.episode),'') ) series
            
            FROM medias as m
                LEFT JOIN ratings on ratings.media_ref = m.id
                LEFT JOIN media_tag_mapping a on a.media_ref = m.id and (a.confidence != -1 or a.confidence IS NULL)
                LEFT JOIN media_people_mapping b on b.media_ref = m.id
                LEFT JOIN media_serie_mapping c on c.media_ref = m.id
            
            
            WHERE id = ?
            GROUP BY m.id")?;
            let row = query.query_row(
            [media_id],Self::row_to_media).optional()?;
            Ok(row)
        }).await?;
        Ok(row)
    }


    pub async fn get_media_source(&self, media_id: &str) -> Result<Option<MediaSource>> {
        let media_id = media_id.to_string();
        let row = self.connection.call( move |conn| { 
            let mut query = conn.prepare("SELECT 
            id, source
            FROM medias
            WHERE id = ?")?;
            let row = query.query_row(
            [media_id],Self::row_to_mediasource).optional()?;
            Ok(row)
        }).await?;
        Ok(row)
    }


    pub async fn update_media(&self, media_id: &str, update: MediaForUpdate) -> Result<()> {
        let id = media_id.to_string();
        let existing = self.get_media(media_id).await?.ok_or_else( || Error::NotFound)?;
        self.connection.call( move |conn| { 
            let mut where_query = QueryBuilder::new();
            /*where_query.add_update(update.name.clone(), QueryWhereType::Equal("name".to_string()));
            where_query.add_update(update.kind, QueryWhereType::Equal("type".to_string()));
            where_query.add_update(update.imdb, QueryWhereType::Equal("imdb".to_string()));
            where_query.add_update(update.slug, QueryWhereType::Equal("slug".to_string()));
            where_query.add_update(update.tmdb, QueryWhereType::Equal("tmdb".to_string()));
            where_query.add_update(update.trakt, QueryWhereType::Equal("trakt".to_string()));
            where_query.add_update(update.tvdb, QueryWhereType::Equal("tvdb".to_string()));
            where_query.add_update(update.otherids, QueryWhereType::Equal("otherids".to_string()));
            where_query.add_update(update.imdb_rating, QueryWhereType::Equal("imdb_rating".to_string()));
            where_query.add_update(update.imdb_votes, QueryWhereType::Equal("imdb_votes".to_string()));
            where_query.add_update(update.trakt_rating, QueryWhereType::Equal("trakt_rating".to_string()));
            where_query.add_update(update.trakt_votes, QueryWhereType::Equal("trakt_votes".to_string()));
            where_query.add_update(update.year, QueryWhereType::Equal("year".to_string()));
            where_query.add_update(update.max_created, QueryWhereType::Equal("maxCreated".to_string()));

            let alts = replace_add_remove_from_array(existing.alt, update.alt, update.add_alts, update.remove_alts);
            where_query.add_update(to_pipe_separated_optional(alts), QueryWhereType::Equal("alt".to_string()));



*/


            where_query.add_where(Some(id), QueryWhereType::Equal("id".to_string()));
            

            let update_sql = format!("UPDATE medias SET {} {}", where_query.format_update(), where_query.format());

            conn.execute(&update_sql, where_query.values())?;
            Ok(())
        }).await?;

        Ok(())
    }

    pub async fn add_media(&self, media: MediaForInsert) -> Result<()> {
        self.connection.call( move |conn| { 

            conn.execute("INSERT INTO medias (id, name, type, alt, params, imdb, slug, tmdb, trakt, tvdb, otherids, year, imdb_rating, imdb_votes, trailer, trakt_rating, trakt_votes)
            VALUES (?, ?, ? ,?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)", params![
                media.id,
                media.media.name,
                media.media.kind,
                //to_pipe_separated_optional(media.alt),
                media.media.params,
                /*media.imdb,
                media.slug,
                media.tmdb,
                media.trakt,
                media.tvdb,
                media.otherids,
                media.year,
                media.imdb_rating,
                media.imdb_votes,
                media.trailer,
                media.trakt_rating,
                media.trakt_votes*/
            ])?;
            
            Ok(())
        }).await?;
        Ok(())
    }

    pub async fn remove_media(&self, media_id: String) -> Result<()> {
        self.connection.call( move |conn| { 
            conn.execute("DELETE FROM medias WHERE id = ?", &[&media_id])?;
            Ok(())
        }).await?;
        Ok(())
    }
}
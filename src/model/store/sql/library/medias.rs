use std::u64;

use rs_plugin_common_interfaces::url::RsLink;
use rusqlite::{params, params_from_iter, types::{FromSql, FromSqlError, FromSqlResult, ToSqlOutput, ValueRef}, OptionalExtension, Row, ToSql};

use crate::{domain::{media::{FileEpisode, FileType, Media, MediaForInsert, MediaForUpdate, MediaItemReference, RsGpsPosition}, MediasIds}, error::RsResult, model::{medias::{MediaQuery, MediaSource, RsSort}, people::PeopleQuery, store::{from_comma_separated_optional, from_pipe_separated_optional, sql::{OrderBuilder, QueryBuilder, QueryWhereType, RsQueryBuilder, SqlOrder, SqlWhereType}, to_comma_separated_optional, to_pipe_separated_optional}, tags::TagQuery}, tools::{array_tools::AddOrSetArray, log::{log_info, LogServiceType}, text_tools::{extract_people, extract_tags}}};
use super::{Result, SqliteLibraryStore};
use crate::model::Error;


impl FromSql for RsGpsPosition {
    fn column_result(value: ValueRef) -> FromSqlResult<Self> {
        String::column_result(value).and_then(|as_string| {
            let mut splitted = as_string.split(",");
            let lat = splitted.next().and_then(|f| f.parse::<f64>().ok()).ok_or(FromSqlError::InvalidType)?;
            let long = splitted.next().and_then(|f| f.parse::<f64>().ok()).ok_or(FromSqlError::InvalidType)?;
            Ok(RsGpsPosition {
                lat,
                long,
            })
        })
    }
}

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

impl RsSort {
    pub fn to_media_query(&self) -> String {
        match self {
            RsSort::Rating => "rating".to_owned(),
            _ => format!("m.{}", self),
        }
    }
}

impl TryFrom<Vec<String>> for MediasIds {
    type Error = crate::Error;
    
    fn try_from(values: Vec<String>) -> RsResult<Self> {
        let mut ids = Self::default();
        for value in values {
            ids.try_add(value)?;
        }
        Ok(ids)
    }
}

impl From<MediasIds> for Vec<String> {
    
    fn from(value: MediasIds) -> Self {
        let mut ids = vec![];
        if let Some(id) = value.as_redseat() {
            ids.push(id)
        }
        if let Some(id) = value.as_imdb() {
            ids.push(id)
        }
        if let Some(id) = value.as_tmdb() {
            ids.push(id.to_string())
        }
        if let Some(id) = value.as_trakt() {
            ids.push(id.to_string())
        }
        if let Some(id) = value.as_tvdb() {
            ids.push(id.to_string())
        }
        ids
    }
}


const MEDIA_QUERY: &str = "SELECT 
        m.id, m.source, m.name, m.description, m.type, m.mimetype, m.size, avg(ratings.rating) as rating, m.md5, m.params, 
        m.width, m.height, m.phash, m.thumbhash, m.focal, m.iso, m.colorSpace, m.sspeed, m.orientation, m.duration, 
        m.acodecs, m.achan, m.vcodecs, m.fps, m.bitrate, m.long, m.lat, m.model, m.pages, m.progress, 
        m.thumb, m.thumbv, m.thumbsize, m.iv, m.origin, m.movie, m.lang, m.uploader, m.uploadkey, m.modified, 
        m.added, m.created,
        
        GROUP_CONCAT(distinct a.tag_ref || '|' || IFNULL(a.confidence, 100)) tags,
        GROUP_CONCAT(distinct b.people_ref) people,
        GROUP_CONCAT(distinct c.serie_ref || '|' || printf('%04d', c.season) || '|' || printf('%04d', c.episode) ) series
        
        FROM medias as m
            LEFT JOIN ratings on ratings.media_ref = m.id
            LEFT JOIN media_tag_mapping a on a.media_ref = m.id and (a.confidence != -1 or a.confidence IS NULL)
            LEFT JOIN media_people_mapping b on b.media_ref = m.id
            LEFT JOIN media_serie_mapping c on c.media_ref = m.id
        
        
        ";

impl SqliteLibraryStore {
    fn row_to_mediasource(row: &Row) -> rusqlite::Result<MediaSource> {
        Ok(MediaSource {
            id: row.get(0)?,
            source: row.get(1)?,
            kind: row.get(2)?,
            thumb_size: row.get(3)?,
            size: row.get(4)?,
            mime: row.get(5)?,
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
            people: from_comma_separated_optional(row.get(43)?),
            series: from_comma_separated_optional(row.get(44)?),
            //series: None,
        })
    }

    pub async fn get_medias(&self, mut query: MediaQuery) -> Result<Vec<Media>> {
        let row = self.connection.call( move |conn| { 


            let mut where_query = QueryBuilder::new();
            
            let sort = query.sort.to_media_query();
            if let Some(page_key) = query.page_key {
                if query.order == SqlOrder::DESC {
                    query.before = Some(page_key);
                } else {
                    query.after = Some(page_key);
                }
            }

            if let Some(q) = &query.after {
                if query.sort == RsSort::Added || query.sort == RsSort::Modified || query.sort == RsSort::Created {
                    where_query.add_where(QueryWhereType::After(&sort, q));
                } else {
                    where_query.add_where(QueryWhereType::After("modified", q));
                }
            }
            if let Some(q) = &query.before {
                if query.sort == RsSort::Added || query.sort == RsSort::Modified || query.sort == RsSort::Created {
                    where_query.add_where(QueryWhereType::Before(&sort, q));
                } else {
                    where_query.add_where(QueryWhereType::Before("modified", q));
                }
            }


            if query.types.len() > 0 {
                let mut types = vec![];
                for kind in &query.types {
                    types.push(QueryWhereType::Equal("type", kind));
                }
                where_query.add_where(QueryWhereType::Or(types));
            }

            for person in &query.people {
                where_query.add_where(QueryWhereType::InStringList("people", ",", person));
            }

            //let series_formated = &query.series.iter().map(|s| format!("{}|", s)).collect::<Vec<String>>();
            for serie in &query.series {
                if serie.contains("|") {
                    where_query.add_where(QueryWhereType::Custom("series like '%' || ? || '%'", serie));
                } else {
                    where_query.add_where(QueryWhereType::Custom("(',' || series || ',' LIKE '%,' || ? || '|%')", serie));
                }
                
            }
            



            
            where_query.add_oder(OrderBuilder::new(sort.to_owned(), query.order));



            for tag in &query.tags {
                where_query.add_recursive("tags", "media_tag_mapping", "media_ref", "tag_ref", tag);
            }
            


            let mut query = conn.prepare(&format!("
            {}
            SELECT 
            m.id, m.source, m.name, m.description, m.type, m.mimetype, m.size,
            (select avg(rating ) from ratings where media_ref = m.id) as rating,
            m.md5, m.params, 
            m.width, m.height, m.phash, m.thumbhash, m.focal, m.iso, m.colorSpace, m.sspeed, m.orientation, m.duration, 
            m.acodecs, m.achan, m.vcodecs, m.fps, m.bitrate, m.long, m.lat, m.model, m.pages, m.progress, 
            m.thumb, m.thumbv, m.thumbsize, m.iv, m.origin, m.movie, m.lang, m.uploader, m.uploadkey, m.modified, 
            m.added, m.created
			,(select GROUP_CONCAT(tag_ref || '|' || IFNULL(confidence, 100)) from media_tag_mapping where media_ref = m.id and (confidence != -1 or confidence IS NULL)) as tags
			,(select GROUP_CONCAT(people_ref ) from media_people_mapping where media_ref = m.id) as people
			,(select GROUP_CONCAT(serie_ref || '|' || printf('%04d', season) || '|' || printf('%04d', episode)) from media_serie_mapping where media_ref = m.id) as series
			
            FROM medias as m
             {}
                          {}
             LIMIT {}", where_query.format_recursive(), where_query.format(), where_query.format_order(), query.limit.unwrap_or(200)))?;

            //println!("query {:?}", query.expanded_sql());


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
            let mut query = conn.prepare(&format!("{} WHERE id = ?", MEDIA_QUERY))?;
            let row = query.query_row(
            [media_id],Self::row_to_media).optional()?;
            Ok(row)
        }).await?;
        Ok(row)
    }

    pub async fn get_media_by_hash(&self, hash: String) -> Option<Media> {
        let row = self.connection.call( move |conn| { 
            let mut query = conn.prepare(&format!("{} WHERE md5 = ?", MEDIA_QUERY))?;
            let row = query.query_row(
            params![hash],Self::row_to_media)?;
            Ok(row)
        }).await;
        row.ok()
    }

    pub async fn get_media_by_origin(&self, origin: RsLink) -> Option<Media> {
        let origin = origin.to_owned();
        let row = self.connection.call( move |conn| { 
            let query_elements = if let Some(file) = origin.file {
                (vec![origin.platform.to_owned(), origin.id.to_owned(), file], "where json_extract(origin, '$.platform') = ? and json_extract(origin, '$.id') = ? and json_extract(origin, '$.file') = ?")
            } else {
                (vec![origin.platform.to_owned(), origin.id.to_owned()], "where json_extract(origin, '$.platform') = ? and json_extract(origin, '$.id') = ?")
            };
            

            let mut query = conn.prepare(&format!("{} {}", MEDIA_QUERY,query_elements.1))?;
            //println!("q {:?}", query.expanded_sql());
            let row = query.query_row(
                params_from_iter(query_elements.0) ,Self::row_to_media)?;
            Ok(row)
        }).await;
        row.ok()
    }


    pub async fn get_media_source(&self, media_id: &str) -> Result<Option<MediaSource>> {
        let media_id = media_id.to_string();
        let row = self.connection.call( move |conn| { 
            let mut query = conn.prepare("SELECT 
            id, source, type, thumbsize, size, mimetype
            FROM medias
            WHERE id = ?")?;
            let row = query.query_row(
            [media_id],Self::row_to_mediasource).optional()?;
            Ok(row)
        }).await?;
        Ok(row)
    }

    pub async fn get_medias_locs(&self, precision: u32) -> Result<Vec<RsGpsPosition>> {
        let rows = self.connection.call( move |conn| { 

            let mut query = conn.prepare("SELECT distinct(round(lat,?) || ',' || round(long,?)) as coord from medias where long IS NOT NULL")?;
            let rows = query.query_map(
            params![precision, precision],|row| {
                let s: RsGpsPosition =  row.get(0)?;
                Ok(s)
            })?;
            let rows:Vec<RsGpsPosition> = rows.collect::<std::result::Result<Vec<RsGpsPosition>, rusqlite::Error>>()?; 
            Ok(rows)
        }).await?;
        Ok(rows)
    }

    pub async fn update_media_thumb(&self, media_id: String) -> Result<()> {
        self.connection.call( move |conn| { 
            conn.execute("update medias set thumbv = ifnull(thumbv, 0) + 1 WHERE id = ?", params![media_id])?;
            Ok(())
        }).await?;
        Ok(())
    }

    pub async fn update_media(&self, media_id: &str, mut update: MediaForUpdate, user_id: Option<String>) -> Result<()> {
        let id = media_id.to_string();
        let existing = self.get_media(media_id).await?.ok_or_else( || Error::NotFound)?;


        if let Some(gps) = &update.gps {
            let splited: Vec<&str> = gps.split(',').map(|t| t.trim()).collect();
            let lat = splited.first().and_then(|s| s.parse::<f64>().ok()).ok_or(Error::ServiceError("updating media".to_owned(), Some(format!("invalid latitude: {}", gps))))?;
            let long = splited.get(1).and_then(|s| s.parse::<f64>().ok()).ok_or(Error::ServiceError("updating media".to_owned(), Some(format!("invalid longitude: {}", gps))))?;
            update.lat = Some(lat);
            update.long = Some(long);
        }
        


        //add tags in description to lookups
        if let Some(description) = &update.description {
            let parsed_tags = extract_tags(&description);
            if parsed_tags.len() > 0 {
                update.tags_lookup.add_or_set(parsed_tags);
            }
        }
        //add people in description to lookups
        if let Some(description) = &update.description {
            let parsed_people = extract_people(&description);
            if parsed_people.len() > 0 {
                update.people_lookup.add_or_set(parsed_people);
            }
        }
        
        // Find tags with lookup 
        if let Some(lookup_tags) = update.tags_lookup {
            let mut found_tags: Vec<MediaItemReference> = vec![];
            for lookup_tag in lookup_tags {
                let found = self.get_tags(TagQuery::new_with_name(&lookup_tag)).await?;
                if let Some(tag) = found.get(0) {
                    found_tags.push(MediaItemReference { id: tag.id.clone(), conf: Some(100) });
                }
            }
            if found_tags.len() > 0 {
                update.add_tags.add_or_set(found_tags);
            }
        }

        if let Some(user) = update.origin.as_ref().and_then(|o| o.user.clone()) {
            println!("user! {user}");
            let mut ppl = update.people_lookup.unwrap_or_default();
            ppl.push(user.to_owned());
            update.people_lookup = Some(ppl);
            println!("user! {:?}", update.people_lookup);
        }
        
        // Find people with lookup 
        if let Some(lookup_people) = update.people_lookup {
            let mut found_people: Vec<MediaItemReference> = vec![];
            for lookup_tag in lookup_people {
                let found = self.get_people(PeopleQuery::from_name(&lookup_tag)).await?;
                if let Some(person) = found.first() {
                    found_people.push(MediaItemReference { id: person.id.clone(), conf: Some(100) });
                }
            }
            if !found_people.is_empty() {
                update.add_people.add_or_set(found_people);
            }
        }
       
        self.connection.call( move |conn| { 
            let mut where_query = QueryBuilder::new();
            

            where_query.add_update(&update.name, "name");
            where_query.add_update(&update.description, "description");
            where_query.add_update(&update.mimetype, "mimetype");
            where_query.add_update(&update.size, "size");
            where_query.add_update(&update.md5, "md5");
            where_query.add_update(&update.created, "created");

            where_query.add_update(&update.width, "width");
            where_query.add_update(&update.height, "height");
            where_query.add_update(&update.color_space, "colorSpace");
            where_query.add_update(&update.bitrate, "bitrate");
            
            let v = to_comma_separated_optional(update.vcodecs);
            where_query.add_update(&v, "vcodecs");
            let v = to_comma_separated_optional(update.acodecs);
            where_query.add_update(&v, "acodecs");


            where_query.add_update(&update.duration, "duration");

            where_query.add_update(&update.progress, "progress");


            where_query.add_update(&update.long, "long");
            where_query.add_update(&update.lat, "lat");

            where_query.add_update(&update.origin, "origin");
            where_query.add_update(&update.movie, "movie");

            where_query.add_update(&update.lang, "lang");

            where_query.add_update(&update.uploader, "uploader");
            where_query.add_update(&update.uploadkey, "uploaderkey");

            where_query.add_where(QueryWhereType::Equal("id", &id));
            if !where_query.columns_update.is_empty() {
                let update_sql = format!("UPDATE medias SET {} {}", where_query.format_update(), where_query.format());
                conn.execute(&update_sql, where_query.values())?;
            }


            if let Some(user_id) = user_id {
                if let Some(rating) = update.rating {

                    conn.execute("INSERT OR REPLACE INTO ratings (media_ref, user_ref, rating) VALUES (? ,? , ?)", params![id, user_id, rating])?;

                }
            }


            let all_tags: Vec<String> = existing.tags.clone().unwrap_or(vec![]).into_iter().filter(|t| t.conf.unwrap_or(1) == 1).map(|t| t.id).collect();
            if let Some(add_tags) = update.add_tags {
                for tag in add_tags {
                    if !all_tags.contains(&tag.id) {
                        let r = conn.execute("INSERT OR REPLACE INTO media_tag_mapping (media_ref, tag_ref, confidence) VALUES (? ,? , ?) ", params![id, tag.id, tag.conf]);
                        if let Err(error) = r {
                            log_info(LogServiceType::Source, format!("unable to add tag {:?}: {:?}", tag, error));
                        }
                    }
                }
            }
            if let Some(remove_tags) = update.remove_tags {
                for tag in remove_tags {
                    conn.execute("DELETE FROM media_tag_mapping WHERE media_ref = ? and tag_ref = ?", params![id, tag])?;
                }
            }


            
            let all_people: Vec<String> = existing.people.clone().unwrap_or(vec![]).into_iter().map(|t| t.id).collect();
            if let Some(add_people) = update.add_people {
                for person in add_people {
                    if !all_people.contains(&person.id) {
                        let r = conn.execute("INSERT INTO media_people_mapping (media_ref, people_ref, confidence) VALUES (? ,? , ?) ", params![id, person.id, person.conf]);
                        if let Err(error) = r {
                            log_info(LogServiceType::Source, format!("unable to add person {:?}: {:?}", person, error));
                        }
                    }
                }
            }
            if let Some(remove_people) = update.remove_people {
                for person in remove_people {
                    conn.execute("DELETE FROM media_people_mapping WHERE media_ref = ? and people_ref = ?", params![id, person])?;
                }
            }
            

            let all_series: Vec<FileEpisode> = existing.series.clone().unwrap_or(vec![]);
            if let Some(add_serie) = update.add_series {
                for file_episode in add_serie {
                    if !all_series.contains(&file_episode) {
                        let r = conn.execute("INSERT INTO media_serie_mapping (media_ref, serie_ref, season, episode) VALUES (? ,? , ?, ?) ", params![id, file_episode.id, file_episode.season, file_episode.episode]);
                        if let Err(error) = r {
                            log_info(LogServiceType::Source, format!("unable to add serie {:?}: {:?}", file_episode, error));
                        }
                    }
                }
            }
            if let Some(remove_series) = update.remove_series {
                for file_serie in remove_series {
                    if let Some(season) = file_serie.season {
                        if let Some(episode) = file_serie.episode {
                            conn.execute("DELETE FROM media_serie_mapping WHERE media_ref = ? and serie_ref = ? and season = ? and episode = ?", params![id, file_serie.id, season, episode])?;
                        } else {
                            conn.execute("DELETE FROM media_serie_mapping WHERE media_ref = ? and serie_ref = ? and season = ?", params![id, file_serie.id, season])?;
                        }
                    } else if let Some(episode) = file_serie.episode {
                        conn.execute("DELETE FROM media_serie_mapping WHERE media_ref = ? and serie_ref = ? and episode = ?", params![id, file_serie.id, episode])?;
                    } else {
                        conn.execute("DELETE FROM media_serie_mapping WHERE media_ref = ? and serie_ref = ?", params![id, file_serie.id])?;
                    }
                   
                }
            }

            Ok(())
        }).await?;

        Ok(())
    }

    pub async fn add_media(&self, insert: MediaForInsert) -> Result<()> {
        self.connection.call( move |conn| { 

            conn.execute("INSERT INTO medias (
            id, source, name, description, type, mimetype, size, md5, params, width, 
            height, phash, thumbhash, focal, iso, colorSpace, sspeed, orientation, duration, acodecs, 
            achan, vcodecs, fps, bitrate, long, lat, model, pages, progress, thumb, 
            thumbv, thumbsize, iv, origin, movie, lang, uploader, uploadkey

            )
            VALUES (?, ?, ? ,?, ?, ?, ?, ?, ?, ?, 
                ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 
                ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 
                ?, ?, ?, ?, ?, ?, ?, ?)", params![
                insert.id,
                insert.media.source,
                insert.media.name,
                insert.media.description,
                insert.media.kind,
                insert.media.mimetype,
                insert.media.size,
                insert.media.md5,
                insert.media.params,
                insert.media.width,

                insert.media.height,
                insert.media.phash,
                insert.media.thumbhash,
                insert.media.focal,
                insert.media.iso,
                insert.media.color_space,
                insert.media.sspeed,
                insert.media.orientation,
                insert.media.duration,
                to_pipe_separated_optional(insert.media.acodecs),
                
                to_pipe_separated_optional(insert.media.achan),
                to_pipe_separated_optional(insert.media.vcodecs),
                insert.media.fps,
                insert.media.bitrate,
                insert.media.long,
                insert.media.lat,
                insert.media.model,
                insert.media.pages,
                insert.media.progress,
                insert.media.thumb,

                insert.media.thumbv,
                insert.media.thumbsize,
                insert.media.iv,
                insert.media.origin,
                insert.media.movie,
                insert.media.lang,
                insert.media.uploader,
                insert.media.uploadkey,
            ])?;
            
            Ok(())
        }).await?;
        Ok(())
    }

    pub async fn remove_media(&self, media_id: String) -> Result<()> {
        self.connection.call( move |conn| { 
            conn.execute("DELETE FROM medias WHERE id = ?", params![media_id])?;
            conn.execute("DELETE FROM ratings WHERE media_ref = ?", params![media_id])?;
            conn.execute("DELETE FROM media_tag_mapping WHERE media_ref = ?", params![media_id])?;
            conn.execute("DELETE FROM media_serie_mapping WHERE media_ref = ?", params![media_id])?;
            conn.execute("DELETE FROM media_people_mapping WHERE media_ref = ?", params![media_id])?;
            conn.execute("DELETE FROM shares WHERE media_ref = ?", params![media_id])?;

            Ok(())
        }).await?;
        Ok(())
    }
}
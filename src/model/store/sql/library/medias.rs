use std::u64;

use chrono::Utc;
use rs_plugin_common_interfaces::{domain::rs_ids::RsIds, url::RsLink};
use rusqlite::{
    params, params_from_iter,
    types::{FromSql, FromSqlError, FromSqlResult, ToSqlOutput, ValueRef},
    OptionalExtension, Row, ToSql,
};
use serde::{Deserialize, Serialize};
use stream_map_any::StreamMapAnyVariant;

use super::{Result, SqliteLibraryStore};
use crate::model::Error;
use crate::{
    domain::{
        library::LibraryLimits,
        media::{
            self, FileEpisode, FileType, Media, MediaForInsert, MediaForUpdate, MediaItemReference,
            RsGpsPosition,
        },
    },
    error::RsResult,
    model::{
        medias::{MediaQuery, MediaSource, RsSort},
        people::PeopleQuery,
        series::SerieQuery,
        store::{
            from_comma_separated_optional, from_pipe_separated_optional,
            sql::{
                OrderBuilder, QueryBuilder, QueryWhereType, RsQueryBuilder, SqlOrder, SqlWhereType,
            },
            to_comma_separated_optional, to_pipe_separated_optional,
        },
        tags::{TagForInsert, TagForUpdate, TagQuery},
    },
    plugins::sources::error::SourcesError,
    tools::{
        array_tools::AddOrSetArray,
        file_tools::{file_type_from_mime, get_mime_from_filename},
        log::{log_info, LogServiceType},
        text_tools::{extract_people, extract_tags},
    },
};
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct MediaBackup {
    pub id: String,
    pub name: String,
    pub size: Option<u64>,
    pub hash: String,
}

impl FromSql for RsGpsPosition {
    fn column_result(value: ValueRef) -> FromSqlResult<Self> {
        String::column_result(value).and_then(|as_string| {
            let mut splitted = as_string.split(",");
            let lat = splitted
                .next()
                .and_then(|f| f.parse::<f64>().ok())
                .ok_or(FromSqlError::InvalidType)?;
            let long = splitted
                .next()
                .and_then(|f| f.parse::<f64>().ok())
                .ok_or(FromSqlError::InvalidType)?;
            Ok(RsGpsPosition { lat, long })
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

const MEDIA_QUERY: &str = "SELECT 
            m.id, m.source, m.name, m.description, m.type, m.mimetype, m.size,
            art.rating as rating,
            m.md5, m.params, 
            m.width, m.height, m.phash, m.thumbhash, m.focal, m.iso, m.colorSpace, m.sspeed, m.orientation, m.duration, 
            m.acodecs, m.achan, m.vcodecs, m.fps, m.bitrate, m.long, m.lat, m.model, m.pages, m.progress, 
            m.thumb, m.thumbv, m.thumbsize, m.iv, m.origin, m.movie, m.book, m.lang, m.uploader, m.uploadkey, m.modified, 
            m.added, m.created
			,(select GROUP_CONCAT(tag_ref || '|' || IFNULL(confidence, 100)) from media_tag_mapping where media_ref = m.id and (confidence != -1 or confidence IS NULL)) as tags
			,(select GROUP_CONCAT(people_ref ) from media_people_mapping where media_ref = m.id) as people
			,(select GROUP_CONCAT(serie_ref || '|' || printf('%04d', season) || '|' || printf('%04d', episode)) from media_serie_mapping where media_ref = m.id) as series,
            m.fnumber, m.icc, m.mp,
			m.progress as user_progress,
			art.rating as user_rating,
            m.originalhash, m.originalid, m.face_recognition_error
			
            FROM medias as m
            LEFT JOIN 
					(SELECT 
							media_ref, 
							AVG(rating) AS rating
						FROM 
							ratings
						GROUP BY 
							media_ref
					) as art
				ON 
					art.media_ref = m.id
        ";

fn media_query(user_id: &Option<String>) -> String {
    if let Some(user_id) = user_id {
        format!("SELECT 
            m.id, m.source, m.name, m.description, m.type, m.mimetype, m.size,
            art.rating as rating,
            m.md5, m.params, 
            m.width, m.height, m.phash, m.thumbhash, m.focal, m.iso, m.colorSpace, m.sspeed, m.orientation, m.duration, 
            m.acodecs, m.achan, m.vcodecs, m.fps, m.bitrate, m.long, m.lat, m.model, m.pages, m.progress, 
            m.thumb, m.thumbv, m.thumbsize, m.iv, m.origin, m.movie, m.book, m.lang, m.uploader, m.uploadkey, m.modified, 
            m.added, m.created
			,(select GROUP_CONCAT(tag_ref || '|' || IFNULL(confidence, 100)) from media_tag_mapping where media_ref = m.id and (confidence != -1 or confidence IS NULL)) as tags
			,(select GROUP_CONCAT(people_ref ) from media_people_mapping where media_ref = m.id) as people
			,(select GROUP_CONCAT(serie_ref || '|' || printf('%04d', season) || '|' || printf('%04d', episode)) from media_serie_mapping where media_ref = m.id) as series,
            m.fnumber, m.icc, m.mp,
			mp.progress as user_progress,
			rt.rating as user_rating,
            m.originalhash, m.originalid, m.face_recognition_error

            FROM medias as m
            LEFT JOIN 
					media_progress mp
				ON 
					mp.media_ref = m.id and mp.user_ref = '{}'
			LEFT JOIN 
					ratings rt
				ON 
					rt.media_ref = m.id and rt.user_ref = '{}'
          LEFT JOIN 
					(SELECT 
							media_ref, 
							AVG(rating) AS rating
						FROM 
							ratings
						GROUP BY 
							media_ref
					) as art
				ON 
					art.media_ref = m.id          
                    
                    
                    ", user_id, user_id)
    } else {
        MEDIA_QUERY.to_string()
    }
}

const MEDIA_BACKUP_QUERY: &str = "SELECT 
            m.id, m.name, m.size, m.md5,
            (select avg(rating ) from ratings where media_ref = m.id) as rating
			,(select GROUP_CONCAT(tag_ref || '|' || IFNULL(confidence, 100)) from media_tag_mapping where media_ref = m.id and (confidence != -1 or confidence IS NULL)) as tags
			,(select GROUP_CONCAT(people_ref ) from media_people_mapping where media_ref = m.id) as people
			,(select GROUP_CONCAT(serie_ref || '|' || printf('%04d', season) || '|' || printf('%04d', episode)) from media_serie_mapping where media_ref = m.id) as series,
            m.fnumber, m.icc, m.mp
			
            FROM medias as m";

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

            avg_rating: row.get(7)?,
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

            progress: row.get(49)?,
            thumb: row.get(30)?,
            thumbv: row.get(31)?,

            thumbsize: row.get(32)?,
            iv: row.get(33)?,

            origin: row.get(34)?,
            movie: row.get(35)?,
            book: row.get(36)?,
            lang: row.get(37)?,
            uploader: row.get(38)?,
            uploadkey: row.get(39)?,

            modified: row.get(40)?,
            added: row.get(41)?,
            created: row.get(42)?,

            tags: from_comma_separated_optional(row.get(43)?),
            people: from_comma_separated_optional(row.get(44)?),
            series: from_comma_separated_optional(row.get(45)?),
            faces: None,
            backups: None,

            f_number: row.get(46)?,
            icc: row.get(47)?,
            mp: row.get(48)?,

            rating: row.get(50)?,

            original_hash: row.get(51)?,
            original_id: row.get(52)?,
            face_recognition_error: row.get(53)?,
            //series: None,
        })
    }

    fn build_media_query(mut query: MediaQuery, limits: LibraryLimits) -> RsQueryBuilder {
        let mut where_query = RsQueryBuilder::new();

        if let Some(text) = query.text {
            let text = format!("%{}%", text);
            where_query.add_where(SqlWhereType::Or(vec![
                SqlWhereType::Like("name".to_owned(), Box::new(text.clone())),
                SqlWhereType::Like("description".to_owned(), Box::new(text.clone())),
            ]));
        }
        let sort = query.sort.to_media_query();

        let mut pagination_handled = false;

        if let Some(page_key_str) = query.page_key {
            //println!("page key {}", page_key_str);
            // Try to split the key into [primary_cursor, id_cursor]
            if let Some((primary, secondary)) = page_key_str.split_once('|') {
                // 1. Process Primary Cursor (Standard behavior)
                if let Ok(primary_val) = primary.parse::<i64>() {
                    // Determine operator based on sort order
                    let op = if query.order == SqlOrder::DESC {
                        "<"
                    } else {
                        ">"
                    };

                    // Construct Tuple Comparison: (sort < val) OR (sort = val AND id < sec_val)
                    // We inject `primary_val` directly (safe as it is an integer)
                    // We bind `secondary` via the parameter (?)
                    let sql = format!(
                        "({col} {op} {val} OR ({col} = {val} AND m.id {op} ?))",
                        col = sort,
                        op = op,
                        val = primary_val
                    );

                    where_query
                        .add_where(SqlWhereType::Custom(sql, Box::new(secondary.to_string())));

                    pagination_handled = true;
                }
                // Mark that we need the secondary sort
            } else {
                // Fallback: No separator, use original logic (single integer)
                if let Ok(val) = page_key_str.parse::<i64>() {
                    if query.order == SqlOrder::DESC {
                        query.before = Some(val);
                    } else {
                        query.after = Some(val);
                    }
                }
            }
        }

        if let Some(q) = query.after {
            if query.sort == RsSort::Added
                || query.sort == RsSort::Modified
                || query.sort == RsSort::Created
            {
                where_query.add_where(SqlWhereType::After(sort.clone(), Box::new(q)));
            } else {
                where_query.add_where(SqlWhereType::After("m.modified".to_owned(), Box::new(q)));
            }
        } else if let Some(q) = query.before {
            if query.sort == RsSort::Added
                || query.sort == RsSort::Modified
                || query.sort == RsSort::Created
            {
                where_query.add_where(SqlWhereType::Before(sort.clone(), Box::new(q)));
            } else {
                where_query.add_where(SqlWhereType::Before("m.modified".to_owned(), Box::new(q)));
            }
        }

        let limit = if let Some(minutes) = limits.delay {
            Some(Utc::now().timestamp_millis() - (minutes * 60000))
        } else {
            None
        };
        if let Some(added) = query.added_before {
            let added = if let Some(limit) = limit {
                added.min(limit)
            } else {
                added
            };
            where_query.add_where(SqlWhereType::Before("m.added".to_owned(), Box::new(added)));
        } else if let Some(limit) = limit {
            where_query.add_where(SqlWhereType::Before("m.added".to_owned(), Box::new(limit)));
        }
        if let Some(added) = query.added_after {
            where_query.add_where(SqlWhereType::After("m.added".to_owned(), Box::new(added)));
        }

        if let Some(added) = query.created_before {
            where_query.add_where(SqlWhereType::Before(
                "m.created".to_owned(),
                Box::new(added),
            ));
        }
        if let Some(added) = query.created_after {
            where_query.add_where(SqlWhereType::After("m.created".to_owned(), Box::new(added)));
        }

        if let Some(added) = query.modified_before {
            where_query.add_where(SqlWhereType::Before(
                "m.modified".to_owned(),
                Box::new(added),
            ));
        }
        if let Some(added) = query.modified_after {
            where_query.add_where(SqlWhereType::After(
                "m.modified".to_owned(),
                Box::new(added),
            ));
        }

        if let Some(long) = query.long {
            let distance = query.distance.map(|d| d * 0.008).unwrap_or(0.1);
            where_query.add_where(SqlWhereType::Between(
                "long".to_owned(),
                Box::new(long - distance),
                Box::new(long + distance),
            ));
        }
        if let Some(lat) = query.lat {
            let distance = query.distance.map(|d| d * 0.008).unwrap_or(0.1);
            where_query.add_where(SqlWhereType::Between(
                "lat".to_owned(),
                Box::new(lat - distance),
                Box::new(lat + distance),
            ));
        }

        if query.gps_square.len() == 4 {
            let longb = query.gps_square.first().unwrap().to_owned();
            let latb = query.gps_square.get(1).unwrap().to_owned();
            let longt = query.gps_square.get(2).unwrap().to_owned();
            let latt = query.gps_square.get(3).unwrap().to_owned();
            where_query.add_where(SqlWhereType::Between(
                "lat".to_owned(),
                Box::new(latb),
                Box::new(latt),
            ));
            where_query.add_where(SqlWhereType::Between(
                "long".to_owned(),
                Box::new(longb),
                Box::new(longt),
            ));
            println!("{} {} {} {}", latb, latt, longb, longt);
        }

        if !query.types.is_empty() {
            let mut types = vec![];
            for kind in query.types {
                types.push(SqlWhereType::Equal("type".to_owned(), Box::new(kind)));
            }
            where_query.add_where(SqlWhereType::Or(types));
        }

        for person in query.people {
            where_query.add_where(SqlWhereType::InStringList(
                "people".to_owned(),
                ",".to_owned(),
                Box::new(person),
            ));
        }

        //let series_formated = &query.series.iter().map(|s| format!("{}|", s)).collect::<Vec<String>>();
        for serie in query.series {
            if serie.contains('|') {
                where_query.add_where(SqlWhereType::Custom(
                    "series like '%' || ? || '%'".to_owned(),
                    Box::new(serie),
                ));
            } else {
                where_query.add_where(SqlWhereType::Custom(
                    "(',' || series || ',' LIKE '%,' || ? || '|%')".to_owned(),
                    Box::new(serie),
                ));
            }
        }

        if let Some(movie) = query.movie {
            where_query.add_where(SqlWhereType::Equal("movie".to_string(), Box::new(movie)));
        }
        if let Some(book) = query.book {
            where_query.add_where(SqlWhereType::Equal("book".to_string(), Box::new(book)));
        }

        if let Some(duration) = query.min_duration {
            where_query.add_where(SqlWhereType::After(
                "duration".to_string(),
                Box::new(duration),
            ));
        }
        if let Some(duration) = query.max_duration {
            where_query.add_where(SqlWhereType::Before(
                "duration".to_string(),
                Box::new(duration),
            ));
        }

        if let Some(size) = query.min_size {
            where_query.add_where(SqlWhereType::GreaterOrEqual(
                "size".to_string(),
                Box::new(size),
            ));
        }
        if let Some(size) = query.max_size {
            where_query.add_where(SqlWhereType::SmallerOrEqual(
                "size".to_string(),
                Box::new(size),
            ));
        }

        if let Some(rating) = query.min_rating {
            where_query.add_where(SqlWhereType::GreaterOrEqual(
                "rating".to_string(),
                Box::new(rating),
            ));
        }
        if let Some(rating) = query.max_rating {
            where_query.add_where(SqlWhereType::SmallerOrEqual(
                "rating".to_string(),
                Box::new(rating),
            ));
        }

        if let Some(codec) = query.vcodec {
            where_query.add_where(SqlWhereType::SeparatedContain(
                "vcodecs".to_string(),
                ",".to_string(),
                Box::new(codec),
            ));
        }

        where_query.add_oder(OrderBuilder::new(sort.to_owned(), query.order.clone()));
        if sort == "rating" {
            where_query.add_oder(OrderBuilder::new("m.added".to_owned(), SqlOrder::DESC));
        }

        where_query.add_oder(OrderBuilder::new("m.id".to_owned(), query.order));

        let tag_filter = query
            .tags_confidence
            .map(|conf| format!(" and (IFNULL(confidence, 100) >= {conf})"));
        for tag in query.tags {
            where_query.add_recursive(
                "tags".to_owned(),
                "media_tag_mapping".to_owned(),
                "media_ref".to_owned(),
                "tag_ref".to_owned(),
                Box::new(tag),
                tag_filter.clone(),
            );
        }

        where_query
    }

    pub async fn get_medias(&self, query: MediaQuery, limits: LibraryLimits) -> Result<Vec<Media>> {
        let row = self
            .connection
            .call(move |conn| {
                let media_raw_query = media_query(&limits.user_id);

                let limit = query.limit.unwrap_or(200);
                let mut where_query = Self::build_media_query(query, limits);

                let mut query = conn.prepare(&format!(
                    "
            {}
            {}
            {}
            {}
             LIMIT {}",
                    where_query.format_recursive(),
                    media_raw_query,
                    where_query.format(),
                    where_query.format_order(),
                    limit
                ))?;

                //println!("query {:?}", query.expanded_sql());

                let rows = query.query_map(where_query.values(), Self::row_to_media)?;
                let backups: Vec<Media> =
                    rows.collect::<std::result::Result<Vec<Media>, rusqlite::Error>>()?;
                Ok(backups)
            })
            .await?;
        Ok(row)
    }

    pub async fn count_medias(&self, query: MediaQuery, limits: LibraryLimits) -> Result<u64> {
        let row = self
            .connection
            .call(move |conn| {
                let limit = query.limit.unwrap_or(200);
                let mut where_query = Self::build_media_query(query, limits);

                let mut query = conn.prepare(&format!(
                    "
            {}
            SELECT 
            count(m.id)
			
            FROM medias as m
             {}
                          {}
             LIMIT {}",
                    where_query.format_recursive(),
                    where_query.format(),
                    where_query.format_order(),
                    limit
                ))?;

                //println!("query {:?}", query.expanded_sql());

                let row: u64 = query.query_row(where_query.values(), |row| row.get(0))?;

                Ok(row)
            })
            .await?;
        Ok(row)
    }

    pub async fn get_media(
        &self,
        media_id: &str,
        user_id: Option<String>,
    ) -> Result<Option<Media>> {
        let media_id = media_id.to_string();
        let row = self
            .connection
            .call(move |conn| {
                let media_raw_query = media_query(&user_id);
                let mut query = conn.prepare(&format!("{} WHERE id = ?", media_raw_query))?;
                let row = query.query_row([media_id], Self::row_to_media).optional()?;
                Ok(row)
            })
            .await?;
        Ok(row)
    }

    pub async fn get_media_by_hash(&self, hash: String, check_original: bool) -> Option<Media> {
        let row = self
            .connection
            .call(move |conn| {
                let media_raw_query = media_query(&None);
                let mut query = if check_original {
                    conn.prepare(&format!(
                        "{} WHERE md5 = ? or originalhash = ?",
                        media_raw_query
                    ))?
                } else {
                    conn.prepare(&format!("{} WHERE md5 = ?", media_raw_query))?
                };
                let row = if check_original {
                    query.query_row(params![hash, hash], Self::row_to_media)?
                } else {
                    query.query_row(params![hash], Self::row_to_media)?
                };
                Ok(row)
            })
            .await;
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
            let media_raw_query = media_query(&None);

            let mut query = conn.prepare(&format!("{} {}", media_raw_query,query_elements.1))?;
            //println!("q {:?}", query.expanded_sql());
            let row = query.query_row(
                params_from_iter(query_elements.0) ,Self::row_to_media)?;
            Ok(row)
        }).await;
        row.ok()
    }

    pub async fn get_media_source(&self, media_id: &str) -> Result<Option<MediaSource>> {
        let media_id = media_id.to_string();
        let row = self
            .connection
            .call(move |conn| {
                let mut query = conn.prepare(
                    "SELECT 
            id, source, type, thumbsize, size, mimetype
            FROM medias
            WHERE id = ?",
                )?;
                let row = query
                    .query_row([media_id], Self::row_to_mediasource)
                    .optional()?;
                Ok(row)
            })
            .await?;
        Ok(row)
    }

    pub async fn get_all_sources(&self) -> Result<Vec<String>> {
        let rows = self
            .connection
            .call(move |conn| {
                let mut query = conn.prepare("SELECT distinct(source) as sourcs from medias")?;
                let rows = query.query_map(params![], |row| {
                    let s: String = row.get(0)?;
                    Ok(s)
                })?;
                let rows: Vec<String> =
                    rows.collect::<std::result::Result<Vec<String>, rusqlite::Error>>()?;
                Ok(rows)
            })
            .await?;
        Ok(rows)
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

    pub async fn get_all_medias_to_backup(
        &self,
        after: i64,
        query: MediaQuery,
    ) -> Result<Vec<MediaBackup>> {
        //println!("mediaquery: {:?}", query);
        let rows = self
            .connection
            .call(move |conn| {
                let mut where_query = Self::build_media_query(query, LibraryLimits::default());
                where_query.add_where(SqlWhereType::After(
                    "(
                    CASE
                        WHEN added >= created AND added >= modified THEN added
                        WHEN created >= modified THEN created
                        ELSE modified
                    END
                )"
                    .to_string(),
                    Box::new(after),
                ));
                let mut query = conn.prepare(&format!(
                    "
        {}
        {}
        {}
        ORDER BY (
                CASE
                    WHEN added >= created AND added >= modified THEN added
                    WHEN created >= modified THEN created
                    ELSE modified
                END
            ) ASC",
                    where_query.format_recursive(),
                    MEDIA_BACKUP_QUERY,
                    where_query.format()
                ))?;
                //format!("query: {:?}", query.expanded_sql());
                let rows = query.query_map(where_query.values(), |row| {
                    let s: MediaBackup = MediaBackup {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        size: row.get(2)?,
                        hash: row.get(3)?,
                    };
                    Ok(s)
                })?;
                let rows: Vec<MediaBackup> =
                    rows.collect::<std::result::Result<Vec<MediaBackup>, rusqlite::Error>>()?;
                Ok(rows)
            })
            .await?;
        Ok(rows)
    }

    pub async fn update_media_thumb(&self, media_id: String) -> Result<()> {
        self.connection
            .call(move |conn| {
                conn.execute(
                    "update medias set thumbv = ifnull(thumbv, 0) + 1 WHERE id = ?",
                    params![media_id],
                )?;
                Ok(())
            })
            .await?;
        Ok(())
    }

    pub async fn update_media(
        &self,
        media_id: &str,
        mut update: MediaForUpdate,
        user_id: Option<String>,
    ) -> Result<()> {
        let id = media_id.to_string();
        let existing = self
            .get_media(media_id, user_id.clone())
            .await?
            .ok_or_else(|| {
                SourcesError::UnableToFindMedia(
                    "store".to_string(),
                    media_id.to_string(),
                    "update_media".to_string(),
                )
            })?;

        if let Some(rename) = &update.name {
            if let Some(mime) = get_mime_from_filename(&rename) {
                update.mimetype = Some(mime.clone());
                update.kind = Some(file_type_from_mime(&mime));
                println!("UPDATE type {:?} {:?}", update.mimetype, update.kind)
            }
        }

        if let Some(gps) = &update.gps {
            let splited: Vec<&str> = gps.split(',').map(|t| t.trim()).collect();
            let lat =
                splited
                    .first()
                    .and_then(|s| s.parse::<f64>().ok())
                    .ok_or(Error::ServiceError(
                        "updating media".to_owned(),
                        Some(format!("invalid latitude: {}", gps)),
                    ))?;
            let long =
                splited
                    .get(1)
                    .and_then(|s| s.parse::<f64>().ok())
                    .ok_or(Error::ServiceError(
                        "updating media".to_owned(),
                        Some(format!("invalid longitude: {}", gps)),
                    ))?;
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
        if let Some(mut lookup_tags) = update.tags_lookup.clone() {
            if let Some(people) = &update.people_lookup {
                for person in people {
                    let person = if person.contains("@") {
                        person.to_string()
                    } else {
                        format!("@{}", person)
                    };
                    lookup_tags.push(person);
                }
            }
            let mut found_tags: Vec<MediaItemReference> = vec![];
            //println!("lookup tags: {:?}", lookup_tags);
            for lookup_tag in lookup_tags {
                let found = self
                    .get_tags(TagQuery::new_with_name(&lookup_tag.replace('#', "")))
                    .await?;
                if let Some(tag) = found.first() {
                    found_tags.push(MediaItemReference {
                        id: tag.id.clone(),
                        conf: Some(100),
                    });
                } else {
                    let tag = self
                        .get_or_create_path(
                            vec!["imported", &lookup_tag.replace('#', "")],
                            TagForUpdate {
                                generated: Some(true),
                                ..Default::default()
                            },
                        )
                        .await?;
                    found_tags.push(MediaItemReference {
                        id: tag.id.clone(),
                        conf: Some(100),
                    });
                }
            }

            //println!("found tags: {:?}", found_tags);
            if found_tags.len() > 0 {
                update.add_tags.add_or_set(found_tags);
            }
        }

        if let Some(user) = update.origin.as_ref().and_then(|o| o.user.clone()) {
            let mut ppl = update.people_lookup.unwrap_or_default();
            ppl.push(user.to_owned());
            update.people_lookup = Some(ppl);
        }

        // Find people with lookup
        if let Some(lookup_people) = update.people_lookup {
            let mut found_people: Vec<MediaItemReference> = vec![];
            for lookup_tag in lookup_people {
                let found = self.get_people(PeopleQuery::from_name(&lookup_tag)).await?;
                if let Some(person) = found.first() {
                    found_people.push(MediaItemReference {
                        id: person.id.clone(),
                        conf: Some(100),
                    });
                }
            }
            if !found_people.is_empty() {
                update.add_people.add_or_set(found_people);
            }
        }

        // Find serie with lookup
        if let Some(lookup_series) = update.series_lookup {
            println!("looking for series {:?}", lookup_series);
            let mut found_series: Vec<FileEpisode> = vec![];
            for lookup_serie in lookup_series {
                let found = self
                    .get_series(SerieQuery {
                        name: Some(lookup_serie),
                        ..Default::default()
                    })
                    .await?;
                if let Some(serie) = found.first() {
                    found_series.push(FileEpisode {
                        id: serie.id.to_string(),
                        season: update.season,
                        episode: update.episode,
                        episode_to: None,
                    });
                }
            }
            println!("Found Series {:?}", found_series);
            if !found_series.is_empty() {
                update.add_series.add_or_set(found_series);
            }
        }

        self.connection.call( move |conn| { 
            let mut where_query = QueryBuilder::new();
            

            where_query.add_update(&update.name, "name");
            where_query.add_update(&update.description, "description");
            where_query.add_update(&update.mimetype, "mimetype");
            where_query.add_update(&update.kind, "type");
            where_query.add_update(&update.size, "size");
            where_query.add_update(&update.md5, "md5");
            where_query.add_update(&update.created, "created");

            where_query.add_update(&update.width, "width");
            where_query.add_update(&update.height, "height");
            where_query.add_update(&update.color_space, "colorSpace");
            where_query.add_update(&update.icc, "icc");
            where_query.add_update(&update.mp, "mp");
            where_query.add_update(&update.fps, "fps");
            where_query.add_update(&update.bitrate, "bitrate");
            where_query.add_update(&update.orientation, "orientation");
            where_query.add_update(&update.iso, "iso");
            where_query.add_update(&update.focal, "focal");
            where_query.add_update(&update.sspeed, "sspeed");
            where_query.add_update(&update.f_number, "fnumber");
            where_query.add_update(&update.model, "model");
            where_query.add_update(&update.pages, "pages");
            
            let v = to_comma_separated_optional(update.vcodecs);
            where_query.add_update(&v, "vcodecs");
            let v = to_comma_separated_optional(update.acodecs);
            where_query.add_update(&v, "acodecs");


            where_query.add_update(&update.duration, "duration");

            //where_query.add_update(&update.progress, "progress");


            where_query.add_update(&update.long, "long");
            where_query.add_update(&update.lat, "lat");

            where_query.add_update(&update.origin, "origin");
            where_query.add_update(&update.movie, "movie");
            where_query.add_update(&update.book, "book");

            where_query.add_update(&update.lang, "lang");

            where_query.add_update(&update.uploader, "uploader");
            where_query.add_update(&update.uploadkey, "uploaderkey");
            
            where_query.add_update(&update.original_hash, "originalhash");
            where_query.add_update(&update.original_id, "originalid");

            where_query.add_where(QueryWhereType::Equal("id", &id));
            if !where_query.columns_update.is_empty() {
                let update_sql = format!("UPDATE medias SET {} {}", where_query.format_update(), where_query.format());
                conn.execute(&update_sql, where_query.values())?;
            }


           /*  if let Some(user_id) = user_id {
                if let Some(rating) = update.rating {
                    conn.execute("INSERT OR REPLACE INTO ratings (media_ref, user_ref, rating) VALUES (? ,? , ?)", params![id, user_id, rating])?;
                }
                if let Some(progress) = update.progress {
                    conn.execute("INSERT OR REPLACE INTO media_progress (media_ref, user_ref, progress) VALUES (? ,? , ?)", params![id, user_id, progress])?;
                }
            }*/
            


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
                    if !all_people.contains(&person.id)  {
                        let r = conn.execute("INSERT OR REPLACE INTO media_people_mapping (media_ref, people_ref, confidence) VALUES (? ,? , ?) ", params![id, person.id, person.conf]);
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
                        let mut current_episode = file_episode.episode.clone();
                        loop {
                            let r = conn.execute("INSERT INTO media_serie_mapping (media_ref, serie_ref, season, episode) VALUES (? ,? , ?, ?) ", params![id, file_episode.id, file_episode.season, current_episode]);
                            if let Err(error) = r {
                                log_info(LogServiceType::Source, format!("unable to add serie {:?}: {:?}", file_episode, error));
                            }
                            if let (Some(current), Some(to)) = (current_episode, file_episode.episode_to) {
                                if current < to {
                                    current_episode = Some(current + 1);
                                } else {
                                    break;
                                }
                            } else {
                                break;
                            }
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
            height, phash, thumbhash, focal, iso, colorSpace, icc, mp, sspeed, fnumber, orientation, duration, acodecs, 
            achan, vcodecs, fps, bitrate, long, lat, model, pages, progress, thumb, 
            thumbv, thumbsize, iv, origin, movie, book, lang, uploader, uploadkey, originalhash, originalid
            )
            VALUES (?, ?, ? ,?, ?, ?, ?, ?, ?, ?, 
                ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?,
                ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 
                ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)", params![
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
                insert.media.icc,
                insert.media.mp,
                insert.media.sspeed,
                insert.media.f_number,
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
                insert.media.book,
                insert.media.lang,
                insert.media.uploader,
                insert.media.uploadkey,

                insert.media.original_hash,
                insert.media.original_id
            ])?;
            
            Ok(())
        }).await?;
        Ok(())
    }

    pub async fn remove_media(&self, media_id: String) -> Result<()> {
        self.connection
            .call(move |conn| {
                conn.execute("DELETE FROM ratings WHERE media_ref = ?", params![media_id])?;
                conn.execute(
                    "DELETE FROM media_tag_mapping WHERE media_ref = ?",
                    params![media_id],
                )?;
                conn.execute(
                    "DELETE FROM media_serie_mapping WHERE media_ref = ?",
                    params![media_id],
                )?;
                conn.execute(
                    "DELETE FROM media_people_mapping WHERE media_ref = ?",
                    params![media_id],
                )?;
                conn.execute("DELETE FROM shares WHERE media_ref = ?", params![media_id])?;
                conn.execute(
                    "DELETE FROM unassigned_faces WHERE media_ref = ?",
                    params![media_id],
                )?;
                conn.execute(
                    "DELETE FROM people_faces WHERE media_ref = ?",
                    params![media_id],
                )?;
                conn.execute("DELETE FROM medias WHERE id = ?", params![media_id])?;

                Ok(())
            })
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::SqliteLibraryStore;
    use crate::domain::media::{FileType, MediaForAdd};
    use crate::model::{medias::MediaQuery, store::sql::SqlOrder};

    #[tokio::test]
    async fn media_book_column_and_filter_roundtrip() {
        let connection = tokio_rusqlite::Connection::open_in_memory().await.unwrap();
        let store = SqliteLibraryStore::new(connection).await.unwrap();
        let media_id = "media-book-link-test".to_string();

        store
            .add_media(
                MediaForAdd {
                    name: "chapter.cbz".to_string(),
                    kind: FileType::Archive,
                    mimetype: "application/vnd.comicbook+zip".to_string(),
                    book: Some("book-42".to_string()),
                    ..Default::default()
                }
                .into_insert_with_id(media_id.clone()),
            )
            .await
            .unwrap();

        let media = store.get_media(&media_id, None).await.unwrap().unwrap();
        assert_eq!(media.book.as_deref(), Some("book-42"));

        let by_book = store
            .get_medias(
                MediaQuery {
                    book: Some("book-42".to_string()),
                    sort: crate::model::medias::RsSort::Added,
                    order: SqlOrder::ASC,
                    ..Default::default()
                },
                crate::domain::library::LibraryLimits::default(),
            )
            .await
            .unwrap();
        assert_eq!(by_book.len(), 1);
        assert_eq!(by_book[0].id, media_id);
    }
}

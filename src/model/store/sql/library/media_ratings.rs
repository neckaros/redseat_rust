use rusqlite::{params, Row};
use super::{Result, SqliteLibraryStore};
use crate::{domain::{deleted::RsDeleted, media_rating::RsMediaRating}, model::{media_ratings::MediaRatingsQuery, store::{sql::{OrderBuilder, RsQueryBuilder, SqlWhereType}, SqliteStore}}};
use rs_plugin_common_interfaces::domain::element_type::element_type_rusqlite;



impl SqliteLibraryStore {
    fn row_to_media_rating(row: &Row) -> rusqlite::Result<RsMediaRating> {
        Ok(RsMediaRating {
            media_ref: row.get(0)?,
            user_ref: row.get(1)?,
            rating: row.get(2)?,
            modified: row.get(3)?
            
        })
    }


    pub async fn get_medias_ratings(&self, query: MediaRatingsQuery, user_ref: String) -> Result<Vec<RsMediaRating>> {
        let row = self.connection.call( move |conn| { 
            let mut where_query = RsQueryBuilder::new();
            if let Some(q) = query.after {
                where_query.add_where(SqlWhereType::After("modified".to_owned(), Box::new(q)));
            }
            if let Some(q) = query.media {
                where_query.add_where(SqlWhereType::Equal("media_ref".to_owned(), Box::new(q)));
            }
            if let Some(q) = query.min_rating {
                where_query.add_where(SqlWhereType::GreaterOrEqual("rating".to_owned(), Box::new(q)));
            }
            
            if let Some(q) = query.max_rating {
                where_query.add_where(SqlWhereType::SmallerOrEqual("rating".to_owned(), Box::new(q)));
            }

            where_query.add_where(SqlWhereType::Equal("user_ref".to_owned(), Box::new(user_ref)));
            where_query.add_oder(OrderBuilder::new("modified".to_owned(), query.order));

            let mut query = conn.prepare(&format!("SELECT media_ref, user_ref, rating, modified FROM ratings {}{}", where_query.format(), where_query.format_order()))?;

            let rows = query.query_map(
            where_query.values(), Self::row_to_media_rating,
            )?;
            let backups:Vec<RsMediaRating> = rows.collect::<std::result::Result<Vec<RsMediaRating>, rusqlite::Error>>()?; 
            Ok(backups)
        }).await?;
        Ok(row)
    }


    pub async fn set_media_rating(&self, media_ref: String, user_ref: String, rating: f64) -> Result<()> {
        self.connection.call( move |conn| { 

            conn.execute("INSERT OR REPLACE INTO ratings (media_ref, user_ref, rating)
            VALUES (?, ? ,?)", params![
                media_ref,
                user_ref,
                rating
            ])?;

            Ok(())
        }).await?;
        Ok(())
    }

}
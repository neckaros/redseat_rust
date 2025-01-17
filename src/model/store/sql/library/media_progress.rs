use rusqlite::{params, Row};
use super::{Result, SqliteLibraryStore};
use crate::{domain::{deleted::RsDeleted, media_progress::RsMediaProgress, media_rating::RsMediaRating}, model::{media_progresses::MediaProgressesQuery, store::{sql::{OrderBuilder, RsQueryBuilder, SqlWhereType}, SqliteStore}}};
use rs_plugin_common_interfaces::domain::element_type::element_type_rusqlite;



impl SqliteLibraryStore {
    fn row_to_media_progress(row: &Row) -> rusqlite::Result<RsMediaProgress> {
        Ok(RsMediaProgress {
            media_ref: row.get(0)?,
            user_ref: row.get(1)?,
            progress: row.get(2)?,
            modified: row.get(3)?
            
        })
    }


    pub async fn get_medias_progresses(&self, query: MediaProgressesQuery) -> Result<Vec<RsMediaProgress>> {
        let row = self.connection.call( move |conn| { 
            let mut where_query = RsQueryBuilder::new();
            if let Some(q) = query.after {
                where_query.add_where(SqlWhereType::After("modified".to_owned(), Box::new(q)));
            }

            where_query.add_oder(OrderBuilder::new("modified".to_owned(), query.order));

            let mut query = conn.prepare(&format!("SELECT media_ref, user_ref, progress, modified FROM media_progress {}{}", where_query.format(), where_query.format_order()))?;

            let rows = query.query_map(
            where_query.values(), Self::row_to_media_progress,
            )?;
            let backups:Vec<RsMediaProgress> = rows.collect::<std::result::Result<Vec<RsMediaProgress>, rusqlite::Error>>()?; 
            Ok(backups)
        }).await?;
        Ok(row)
    }

}
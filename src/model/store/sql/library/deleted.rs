use rusqlite::{params, Row};
use super::{Result, SqliteLibraryStore};
use crate::{domain::deleted::RsDeleted, model::{deleted::DeletedQuery, store::{sql::{OrderBuilder, RsQueryBuilder, SqlWhereType}, SqliteStore}}};
use rs_plugin_common_interfaces::domain::element_type::element_type_rusqlite;



impl SqliteLibraryStore {
    fn row_to_deleted(row: &Row) -> rusqlite::Result<RsDeleted> {
        Ok(RsDeleted {
            kind: row.get(0)?,
            id: row.get(1)?,
            date: row.get(2)?,
            
        })
    }


    pub async fn get_deleted(&self, query: DeletedQuery) -> Result<Vec<RsDeleted>> {
        let row = self.connection.call( move |conn| { 
            let mut where_query = RsQueryBuilder::new();
            if let Some(q) = query.after {
                where_query.add_where(SqlWhereType::After("date".to_owned(), Box::new(q)));
            }

            where_query.add_oder(OrderBuilder::new("date".to_owned(), query.order));

            let mut query = conn.prepare(&format!("SELECT type, id, date FROM deleted {}{}", where_query.format(), where_query.format_order()))?;

            let rows = query.query_map(
            where_query.values(), Self::row_to_deleted,
            )?;
            let backups:Vec<RsDeleted> = rows.collect::<std::result::Result<Vec<RsDeleted>, rusqlite::Error>>()?; 
            Ok(backups)
        }).await?;
        Ok(row)
    }


    pub async fn add_deleted(&self, deleted: RsDeleted) -> Result<()> {
        self.connection.call( move |conn| { 

            conn.execute("INSERT OR REPLACE INTO deleted (type, id, date)
            VALUES (?, ? ,?, ?)", params![
                deleted.kind,
                deleted.id,
                deleted.date
            ])?;

            Ok(())
        }).await?;
        Ok(())
    }


}
use std::str::FromStr;

use rusqlite::{params, params_from_iter, types::{FromSql, FromSqlError, FromSqlResult, ToSqlOutput, ValueRef}, OptionalExtension, Row, ToSql};

use crate::{domain::{backup::Backup, credential::{Credential, CredentialType}, tag::Tag}, model::{backups::BackupForUpdate, credentials::CredentialForUpdate, store::{from_comma_separated, from_comma_separated_optional, sql::{QueryBuilder, QueryWhereType}, to_comma_separated, to_comma_separated_optional, SqliteStore}, tags::{TagForUpdate, TagQuery}}, tools::log::log_info};
use super::{Result, SqliteLibraryStore};



impl SqliteLibraryStore {
  
    fn row_to_tag(row: &Row) -> rusqlite::Result<Tag> {
        Ok(Tag {
            id: row.get(0)?,
            name: row.get(1)?,
            parent: row.get(2)?,
            kind: row.get(3)?,
            alt: from_comma_separated_optional(row.get(4)?),
            thumb: row.get(5)?,
            params: row.get(6)?,
            modified: row.get(7)?,
            added: row.get(8)?,
            generated: row.get(9)?,
            path: row.get(10)?,
        })
    }

    pub async fn get_tags(&self, query: TagQuery) -> Result<Vec<Tag>> {
        let row = self.connection.call( move |conn| { 
            let mut where_query = QueryBuilder::new();
            where_query.add_where(query.path, QueryWhereType::Like("path".to_string()));


            let mut query = conn.prepare(&format!("SELECT id, name, parent, type, alt, thumb, params, modified, added, generated, path  FROM tags {}", where_query.format()))?;
            let rows = query.query_map(
            where_query.values(), Self::row_to_tag,
            )?;
            let backups:Vec<Tag> = rows.collect::<std::result::Result<Vec<Tag>, rusqlite::Error>>()?; 
            Ok(backups)
        }).await?;
        Ok(row)
    }
    pub async fn get_tag(&self, credential_id: &str) -> Result<Option<Tag>> {
        let credential_id = credential_id.to_string();
        let row = self.connection.call( move |conn| { 
            let mut query = conn.prepare("SELECT id, name, parent, type, alt, thumb, params, modified, added, generated, path FROM tags WHERE id = ?")?;
            let row = query.query_row(
            [credential_id],Self::row_to_tag).optional()?;
            Ok(row)
        }).await?;
        Ok(row)
    }



    pub async fn update_tag(&self, id: &str, update: TagForUpdate) -> Result<()> {

        let id = id.to_string();
        self.connection.call( move |conn| { 

            let mut where_query = QueryBuilder::new();
            where_query.add_update(update.name, QueryWhereType::Equal("name".to_string()));
            where_query.add_update(update.parent, QueryWhereType::Equal("parent".to_string()));
            where_query.add_update(update.kind, QueryWhereType::Equal("kind".to_string()));
            where_query.add_update(to_comma_separated_optional(update.alt), QueryWhereType::Equal("alt".to_string()));
            where_query.add_update(update.thumb, QueryWhereType::Equal("thumb".to_string()));
            where_query.add_update(update.params, QueryWhereType::Equal("params".to_string()));
            where_query.add_update(update.generated, QueryWhereType::Equal("generated".to_string()));
            where_query.add_update(update.path, QueryWhereType::Equal("path".to_string()));

            where_query.add_where(Some(id), QueryWhereType::Equal("id".to_string()));

            let update_sql = format!("UPDATE Tags SET {} {}", where_query.format_update(), where_query.format());

            conn.execute(&update_sql, where_query.values())?;
            
            Ok(())
        }).await?;
        Ok(())
    }
/*
    pub async fn add_backup(&self, backup: Backup) -> Result<()> {
        self.server_store.call( move |conn| { 

            conn.execute("INSERT INTO Backups (id, source, credentials, library, path, schedule, filter, last, password, size)
            VALUES (?, ?, ? ,?, ?, ?, ?, ?, ?, ?)", params![
                backup.id,
                backup.source,
                backup.credentials,
                backup.library,
                backup.path,
                backup.schedule,
                backup.filter,
                backup.last,
                backup.password,
                backup.size
            ])?;
            
            Ok(())
        }).await?;
        Ok(())
    }

    pub async fn remove_backup(&self, credential_id: String) -> Result<()> {
        self.server_store.call( move |conn| { 
            conn.execute("DELETE FROM Backups WHERE id = ?", &[&credential_id])?;
            Ok(())
        }).await?;
        Ok(())
    }*/
}
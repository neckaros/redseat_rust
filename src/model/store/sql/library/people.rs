use std::str::FromStr;

use rusqlite::{params, types::FromSqlError, OptionalExtension, Row};
use serde_json::Value;

use crate::{domain::people::Person, model::{store::{from_pipe_separated_optional, sql::{OrderBuilder, QueryBuilder, QueryWhereType, SqlOrder}, to_pipe_separated_optional}, tags::{TagForInsert, TagForUpdate, TagQuery}}};
use super::{Result, SqliteLibraryStore};
use crate::model::Error;



impl SqliteLibraryStore {
  
    fn row_to_person(row: &Row) -> rusqlite::Result<Person> {
        let socials: Value = row.get(2)?;
        Ok(Person {
            id: row.get(0)?,
            name: row.get(1)?,
            socials: serde_json::from_value(socials).map_err(|_| FromSqlError::InvalidType)?,
            kind: row.get(3)?,
            alt: from_pipe_separated_optional(row.get(4)?),
            portrait: row.get(5)?,
            params: row.get(6)?,
            birthday: row.get(7)?,
            modified: row.get(8)?,
            added: row.get(9)?,
        })
    }

    pub async fn get_people(&self, query: TagQuery) -> Result<Vec<Person>> {
        let row = self.connection.call( move |conn| { 
            let mut where_query = QueryBuilder::new();
            where_query.add_where(query.path, QueryWhereType::Like("path".to_string()));
            where_query.add_where(query.after, QueryWhereType::After("modified".to_string()));
            if query.after.is_some() {
                where_query.add_oder(OrderBuilder::new("modified".to_string(), SqlOrder::ASC))
            }


            let mut query = conn.prepare(&format!("SELECT id, name, parent, type, alt, thumb, params, modified, added, generated, path  FROM tags {}{}", where_query.format(), where_query.format_order()))?;
            let rows = query.query_map(
            where_query.values(), Self::row_to_person,
            )?;
            let backups:Vec<Person> = rows.collect::<std::result::Result<Vec<Person>, rusqlite::Error>>()?; 
            Ok(backups)
        }).await?;
        Ok(row)
    }
    pub async fn get_person(&self, credential_id: &str) -> Result<Option<Person>> {
        let credential_id = credential_id.to_string();
        let row = self.connection.call( move |conn| { 
            let mut query = conn.prepare("SELECT id, name, parent, type, alt, thumb, params, modified, added, generated, path FROM tags WHERE id = ?")?;
            let row = query.query_row(
            [credential_id],Self::row_to_person).optional()?;
            Ok(row)
        }).await?;
        Ok(row)
    }



    pub async fn update_person(&self, tag_id: &str, update: TagForUpdate) -> Result<()> {
        let id = tag_id.to_string();
        self.connection.call( move |conn| { 
            let mut where_query = QueryBuilder::new();
            where_query.add_update(update.name.clone(), QueryWhereType::Equal("name".to_string()));
            where_query.add_update(update.parent.clone(), QueryWhereType::Equal("parent".to_string()));
            where_query.add_update(update.kind, QueryWhereType::Equal("kind".to_string()));
            where_query.add_update(to_pipe_separated_optional(update.alt), QueryWhereType::Equal("alt".to_string()));
            where_query.add_update(update.thumb, QueryWhereType::Equal("thumb".to_string()));
            where_query.add_update(update.params, QueryWhereType::Equal("params".to_string()));
            where_query.add_update(update.generated, QueryWhereType::Equal("generated".to_string()));

            where_query.add_where(Some(id), QueryWhereType::Equal("id".to_string()));
            

            let update_sql = format!("UPDATE Tags SET {} {}", where_query.format_update(), where_query.format());

            conn.execute(&update_sql, where_query.values())?;
            Ok(())
        }).await?;

        Ok(())
    }

    pub async fn add_person(&self, tag: TagForInsert) -> Result<()> {
        self.connection.call( move |conn| { 

            conn.execute("INSERT INTO tags (id, name, parent, type, alt, thumb, params, generated, path)
            VALUES (?, ?, ? ,?, ?, ?, ?, ?, ?)", params![
                tag.id,
                tag.name,
                tag.parent,
                tag.kind,
                to_pipe_separated_optional(tag.alt),
                tag.thumb,
                tag.params,
                tag.generated
            ])?;
            
            Ok(())
        }).await?;
        Ok(())
    }

    pub async fn remove_person(&self, tag_id: String) -> Result<()> {
        self.connection.call( move |conn| { 
            conn.execute("DELETE FROM tags WHERE id = ?", &[&tag_id])?;
            Ok(())
        }).await?;
        Ok(())
    }
}
use rusqlite::{params, OptionalExtension, Row};

use crate::{domain::tag::Tag, model::{store::{from_pipe_separated_optional, sql::{OrderBuilder, QueryBuilder, QueryWhereType, SqlOrder}, to_pipe_separated_optional}, tags::{TagForInsert, TagForUpdate, TagQuery}}};
use super::{Result, SqliteLibraryStore};
use crate::model::Error;


impl SqliteLibraryStore {
  
    fn row_to_tag(row: &Row) -> rusqlite::Result<Tag> {
        Ok(Tag {
            id: row.get(0)?,
            name: row.get(1)?,
            parent: row.get(2)?,
            kind: row.get(3)?,
            alt: from_pipe_separated_optional(row.get(4)?),
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
            where_query.add_where(query.after, QueryWhereType::After("modified".to_string()));
            if query.after.is_some() {
                where_query.add_oder(OrderBuilder::new("modified".to_string(), SqlOrder::ASC))
            }


            let mut query = conn.prepare(&format!("SELECT id, name, parent, type, alt, thumb, params, modified, added, generated, path  FROM tags {}{}", where_query.format(), where_query.format_order()))?;
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



    pub async fn update_tag(&self, tag_id: &str, update: TagForUpdate) -> Result<()> {
        let id = tag_id.to_string();
        let existing_tag = self.get_tag(&tag_id).await?.ok_or_else(|| Error::NotFound)?;
        self.connection.call( move |conn| { 
            let tx = conn.transaction()?;
            let mut where_query = QueryBuilder::new();
            where_query.add_update(update.name.clone(), QueryWhereType::Equal("name".to_string()));
            where_query.add_update(update.parent.clone(), QueryWhereType::Equal("parent".to_string()));
            where_query.add_update(update.kind, QueryWhereType::Equal("type".to_string()));
            where_query.add_update(to_pipe_separated_optional(update.alt), QueryWhereType::Equal("alt".to_string()));
            where_query.add_update(update.thumb, QueryWhereType::Equal("thumb".to_string()));
            where_query.add_update(update.params, QueryWhereType::Equal("params".to_string()));
            where_query.add_update(update.generated, QueryWhereType::Equal("generated".to_string()));

            where_query.add_where(Some(id), QueryWhereType::Equal("id".to_string()));
            

            let update_sql = format!("UPDATE Tags SET {} {}", where_query.format_update(), where_query.format());

            tx.execute(&update_sql, where_query.values())?;
            
            if let Some(new_name) = &update.name {
                tx.execute("UPDATE tags SET path = REPLACE(path, ?, ?) where path like ?", params![existing_tag.childs_path(), format!("{}{}/", existing_tag.path, new_name), existing_tag.childs_path()])?;
            } 
            if let Some(new_parent) = update.parent {
                let mut query_parent = tx.prepare("SELECT id, name, parent, type, alt, thumb, params, modified, added, generated, path FROM tags WHERE id = ?")?;
                let parent = query_parent.query_row(&[&new_parent],Self::row_to_tag)?;
                
                tx.execute("UPDATE tags SET path = ? where id = ?", params![parent.childs_path(), &existing_tag.id])?;

                tx.execute("UPDATE tags SET path = REPLACE(path, ?, ?) where path like ?", params![existing_tag.childs_path(), format!("{}{}/", parent.childs_path(), &existing_tag.name), existing_tag.childs_path()])?;
            } 

            tx.commit()?;
            Ok(())
        }).await?;

        Ok(())
    }

    pub async fn add_tag(&self, tag: TagForInsert) -> Result<()> {
        self.connection.call( move |conn| { 
            let new_path = if let Some(parent) = &tag.parent {
                let mut query_parent = conn.prepare("SELECT id, name, parent, type, alt, thumb, params, modified, added, generated, path FROM tags WHERE id = ?")?;
                let parent = query_parent.query_row(&[&parent],Self::row_to_tag)?;
                parent.childs_path()
            } else {
                String::from("/")
            };
            
            conn.execute("INSERT INTO tags (id, name, parent, type, alt, thumb, params, generated, path)
            VALUES (?, ?, ? ,?, ?, ?, ?, ?, ?)", params![
                tag.id,
                tag.name,
                tag.parent,
                tag.kind,
                to_pipe_separated_optional(tag.alt),
                tag.thumb,
                tag.params,
                tag.generated,
                new_path
            ])?;
            
            Ok(())
        }).await?;
        Ok(())
    }

    pub async fn remove_tag(&self, tag_id: String) -> Result<()> {
        self.connection.call( move |conn| { 
            let tx = conn.transaction()?;
            
            let existing = tx.query_row("SELECT id, name, parent, type, alt, thumb, params, modified, added, generated, path FROM tags WHERE id = ?", &[&tag_id],Self::row_to_tag)?;

            tx.execute("DELETE FROM tags WHERE id = ?", &[&tag_id])?;
            tx.execute("DELETE FROM media_tag_mapping  WHERE tag_ref = ?", &[&tag_id])?;
            tx.execute("DELETE FROM tags WHERE path like ?", &[&format!("{}%", existing.childs_path())])?;

            tx.commit()?;
            Ok(())
        }).await?;
        Ok(())
    }
}
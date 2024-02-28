

use rusqlite::{params, types::FromSqlError, OptionalExtension, Row};


use crate::{domain::people::Person, model::{people::{PeopleQuery, PersonForInsert, PersonForUpdate}, store::{from_pipe_separated_optional, sql::{deserialize_from_row, OrderBuilder, QueryBuilder, QueryWhereType, SqlOrder}, to_pipe_separated_optional}}, tools::{array_tools::replace_add_remove_from_array, serialization::optional_serde_to_string}};
use super::{Result, SqliteLibraryStore};
use crate::model::Error;



impl SqliteLibraryStore {
  
    fn row_to_person(row: &Row) -> rusqlite::Result<Person> {
        Ok(Person {
            id: row.get(0)?,
            name: row.get(1)?,
            socials: deserialize_from_row(row, 2).map_err(|_| FromSqlError::InvalidType)?,
            kind: row.get(3)?,
            alt: from_pipe_separated_optional(row.get(4)?),
            portrait: row.get(5)?,
            params: row.get(6)?,
            birthday: row.get(7)?,
            modified: row.get(8)?,
            added: row.get(9)?,
        })
    }

    pub async fn get_people(&self, query: PeopleQuery) -> Result<Vec<Person>> {
        let row = self.connection.call( move |conn| { 
            let mut where_query = QueryBuilder::new();
            where_query.add_where(query.after, QueryWhereType::After("modified".to_string()));
            if query.after.is_some() {
                where_query.add_oder(OrderBuilder::new("modified".to_string(), SqlOrder::ASC))
            }


            let mut query = conn.prepare(&format!("SELECT id, name, socials, type, alt, portrait, params, birthday, modified, added  FROM people {}{}", where_query.format(), where_query.format_order()))?;
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
            let mut query = conn.prepare("SELECT id, name, socials, type, alt, portrait, params, birthday, modified, added FROM people WHERE id = ?")?;
            let row = query.query_row(
            [credential_id],Self::row_to_person).optional()?;
            Ok(row)
        }).await?;
        Ok(row)
    }



    pub async fn update_person(&self, person_id: &str, update: PersonForUpdate) -> Result<()> {
        let id = person_id.to_string();

        let existing = self.get_person(&person_id).await?.ok_or_else( || Error::NotFound)?;


        self.connection.call( move |conn| { 
            let mut where_query = QueryBuilder::new();
            where_query.add_update(update.name.clone(), QueryWhereType::Equal("name".to_string()));

            where_query.add_update(update.kind, QueryWhereType::Equal("type".to_string()));
            where_query.add_update(update.portrait, QueryWhereType::Equal("portrait".to_string()));
            where_query.add_update(update.params, QueryWhereType::Equal("params".to_string()));
            where_query.add_update(update.birthday, QueryWhereType::Equal("birthday".to_string()));

            let alts = replace_add_remove_from_array(existing.alt, update.alt, update.add_alts, update.remove_alts);
            where_query.add_update(to_pipe_separated_optional(alts), QueryWhereType::Equal("alt".to_string()));

            let socials = replace_add_remove_from_array(existing.socials, update.socials, update.add_socials, update.remove_socials);
            let socials = optional_serde_to_string(socials).map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
            where_query.add_update(socials, QueryWhereType::Equal("socials".to_string()));

            where_query.add_where(Some(id), QueryWhereType::Equal("id".to_string()));
            

            let update_sql = format!("UPDATE people SET {} {}", where_query.format_update(), where_query.format());

            conn.execute(&update_sql, where_query.values())?;
            Ok(())
        }).await?;

        Ok(())
    }

    pub async fn add_person(&self, person: PersonForInsert) -> Result<()> {
        self.connection.call( move |conn| { 
            let socials = if let Some(soc) = person.socials {
                Some(serde_json::to_string(&soc).map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?)
            } else {
                None
            };
            conn.execute("INSERT INTO people (id, name, socials, type, alt, portrait, params, birthday)
            VALUES (?, ?, ? ,?, ?, ?, ?, ?)", params![
                person.id,
                person.name,
                socials,
                person.kind,
                to_pipe_separated_optional(person.alt),
                person.portrait,
                person.params,
                person.birthday
            ])?;
            
            Ok(())
        }).await?;
        Ok(())
    }

    pub async fn remove_person(&self, tag_id: String) -> Result<()> {
        self.connection.call( move |conn| { 
            conn.execute("DELETE FROM people WHERE id = ?", &[&tag_id])?;
            Ok(())
        }).await?;
        Ok(())
    }
}
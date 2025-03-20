

use rs_plugin_common_interfaces::domain::rs_ids::RsIds;
use rusqlite::{params, types::FromSqlError, OptionalExtension, Row};


use crate::{domain::people::Person, model::{people::{PeopleQuery, PersonForInsert, PersonForUpdate}, store::{from_pipe_separated_optional, sql::{deserialize_from_row, OrderBuilder, QueryBuilder, QueryWhereType, RsQueryBuilder, SqlOrder, SqlWhereType}, to_pipe_separated_optional}}, tools::{array_tools::replace_add_remove_from_array, serialization::optional_serde_to_string}};
use super::{Result, SqliteLibraryStore};
use crate::model::Error;




impl SqliteLibraryStore {

    const PEOPLE_FIELDS: &str = "id, name, socials, type, alt, portrait, params, birthday, modified, added, posterv, generated, imdb, slug, tmdb, trakt, death, gender, country, bio";
  
    fn row_to_person(row: &Row) -> rusqlite::Result<Person> {
        Ok(Person {
            id: row.get(0)?,
            name: row.get(1)?,
            socials: deserialize_from_row(row, 2)?,
            kind: row.get(3)?,
            alt: from_pipe_separated_optional(row.get(4)?),
            portrait: row.get(5)?,
            params: row.get(6)?,
            birthday: row.get(7)?,
            modified: row.get(8)?,
            added: row.get(9)?,
            posterv: row.get(10)?,
            generated: row.get(11)?,

            imdb: row.get(12)?,
            slug: row.get(13)?,
            tmdb: row.get(14)?,
            trakt: row.get(15)?,

            death: row.get(16)?,
            gender: row.get(17)?,
            country: row.get(18)?,
            bio: row.get(19)?,
        })
    }


    pub async fn get_people(&self, query: PeopleQuery) -> Result<Vec<Person>> {
        let row = self.connection.call( move |conn| { 
            let mut where_query = RsQueryBuilder::new();
            if let Some(q) = query.after {
                where_query.add_where(SqlWhereType::After("modified".to_owned(), Box::new(q)));
            }
            if query.after.is_some() {
                where_query.add_oder(OrderBuilder::new("modified".to_string(), SqlOrder::ASC))
            } else {
                if query.name.is_some() {
                    where_query.add_oder(OrderBuilder::new("score".to_string(), SqlOrder::DESC));
                }
                where_query.add_oder(OrderBuilder::new("name".to_string(), SqlOrder::ASC))
            }
            

            let mut score = "".to_string();
            if let Some(q) = query.name {

                score = format!(",
(case 
when name = '{}' then 100 
when socials like '%\"id\":\"{}\"%'  then 20
when (alt like '%|{}|%' or  alt like '{}|%'  or  alt like '%|{}'  or alt = '{}' COLLATE NOCASE ) then 10
else 0 end) as score", q, q, q, q, q, q);
                let name_queries = vec![SqlWhereType::EqualWithAlt("name".to_owned(), "alt".to_owned(), "|".to_owned(), Box::new(q.clone())),
                SqlWhereType::Like("socials".to_owned(), Box::new(format!("%\"id\":\"{}\"%", q)))];
                where_query.add_where(SqlWhereType::Or(name_queries));

                
            }

            let mut query = conn.prepare(&format!("SELECT {}{}  FROM people {}{}", Self::PEOPLE_FIELDS, score, where_query.format(), where_query.format_order()))?;

            //println!("sql: {:?}", query.expanded_sql());

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
            let mut query = conn.prepare(&format!("SELECT {} FROM people WHERE id = ?", Self::PEOPLE_FIELDS))?;
            let row = query.query_row(
            [credential_id],Self::row_to_person).optional()?;
            Ok(row)
        }).await?;
        Ok(row)
    }

    pub async fn get_person_by_external_id(&self, ids: RsIds) -> Result<Option<Person>> {
        
        //println!("{}, {}, {}, {}, {}",i.imdb.unwrap_or("zz".to_string()), i.slug.unwrap_or("zz".to_string()), i.tmdb.unwrap_or(0), i.trakt.unwrap_or(0), i.tvdb.unwrap_or(0));
        let row = self.connection.call( move |conn| { 
            let mut query = conn.prepare(&format!("SELECT  
            {}
            FROM people 
            WHERE 
            imdb = ? or slug = ? or tmdb = ? or trakt = ?", Self::PEOPLE_FIELDS))?;
            let row = query.query_row(
            params![ids.imdb.unwrap_or("zz".to_string()), ids.slug.unwrap_or("zz".to_string()), ids.tmdb.unwrap_or(0), ids.trakt.unwrap_or(0)],Self::row_to_person).optional()?;
            Ok(row)
        }).await?;
        Ok(row)
    }


    pub async fn update_person(&self, person_id: &str, update: PersonForUpdate) -> Result<()> {
        let id = person_id.to_string();

        let existing = self.get_person(&person_id).await?.ok_or_else( || Error::NotFound)?;


        self.connection.call( move |conn| { 
            let mut where_query = QueryBuilder::new();

            where_query.add_update(&update.name, "name");
            where_query.add_update(&update.kind, "type");
            where_query.add_update(&update.portrait, "portrait");
            where_query.add_update(&update.params, "params");
            where_query.add_update(&update.birthday, "birthday");
            where_query.add_update(&update.generated, "generated");
            
            where_query.add_update(&update.imdb, "imdb");
            where_query.add_update(&update.slug, "slug");
            where_query.add_update(&update.tmdb, "tmdb");
            where_query.add_update(&update.trakt, "trakt");

            
            
            where_query.add_update(&update.bio, "bio");
            where_query.add_update(&update.gender, "gender");
            where_query.add_update(&update.death, "death");
            where_query.add_update(&update.country, "country");

            let alts = replace_add_remove_from_array(existing.alt, update.alt, update.add_alts, update.remove_alts);
            let v = to_pipe_separated_optional(alts);
            where_query.add_update(&v, "alt");
            println!("socialtsdd {:?}", v);

            let socials = replace_add_remove_from_array(existing.socials, update.socials, update.add_socials, update.remove_socials);
            let socials = optional_serde_to_string(socials).map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
            where_query.add_update(&socials, "socials");

            where_query.add_where(QueryWhereType::Equal("id", &id));
            

            let update_sql = format!("UPDATE people SET {} {}", where_query.format_update(), where_query.format());

            conn.execute(&update_sql, where_query.values())?;
            Ok(())
        }).await?;

        Ok(())
    }

    pub async fn update_person_portrait(&self, id: String) -> Result<()> {
        self.connection.call( move |conn| { 
            conn.execute("update people set posterv = ifnull(posterv, 0) + 1 WHERE id = ?", params![id])?;
            Ok(())
        }).await?;
        Ok(())
    }

    pub async fn add_person(&self, person: PersonForInsert) -> Result<()> {
        self.connection.call( move |conn| { 
            
            let id = person.id;
            let person = person.person;
            let socials = if let Some(soc) = person.socials {
                Some(serde_json::to_string(&soc).map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?)
            } else {
                None
            };

            conn.execute("INSERT INTO people (id, name, socials, type, alt, portrait, params, birthday, generated, imdb, slug, tmdb, trakt, death, gender, country, bio)
            VALUES (?, ?, ? ,?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)", params![
                id,
                person.name,
                socials,
                person.kind,
                to_pipe_separated_optional(person.alt),
                person.portrait,
                person.params,
                person.birthday,
                person.generated,

                person.imdb,
                person.slug,
                person.tmdb,
                person.trakt,
                
                person.death,
                person.gender,
                person.country,
                person.bio
                
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
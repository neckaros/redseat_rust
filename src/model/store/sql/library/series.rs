use rusqlite::{params, OptionalExtension, Row};

use crate::{domain::serie::Serie, model::{series::{SerieForInsert, SerieForUpdate, SerieQuery}, store::{from_pipe_separated_optional, sql::{OrderBuilder, QueryBuilder, QueryWhereType, SqlOrder}, to_pipe_separated_optional}}, tools::array_tools::replace_add_remove_from_array};
use super::{Result, SqliteLibraryStore};
use crate::model::Error;



impl SqliteLibraryStore {
  
    fn row_to_serie(row: &Row) -> rusqlite::Result<Serie> {
        Ok(Serie {
            id: row.get(0)?,
            name: row.get(1)?,
            kind: row.get(2)?,
            alt: from_pipe_separated_optional(row.get(3)?),
            params: row.get(4)?,

            imdb: row.get(5)?,
            slug: row.get(6)?,
            tmdb: row.get(7)?,
            trakt: row.get(8)?,
            tvdb: row.get(9)?,

            otherids: row.get(10)?,
            year: row.get(11)?,
            modified: row.get(12)?,
            added: row.get(13)?,

            imdb_rating: row.get(14)?,
            imdb_votes: row.get(15)?,

            trailer: row.get(16)?,
            max_created: row.get(17)?,
            trakt_rating: row.get(18)?,
            trakt_votes: row.get(19)?,
        })
    }

    pub async fn get_series(&self, query: SerieQuery) -> Result<Vec<Serie>> {
        let row = self.connection.call( move |conn| { 
            let mut where_query = QueryBuilder::new();
            where_query.add_where(query.after, QueryWhereType::After("modified".to_string()));
            if query.after.is_some() {
                where_query.add_oder(OrderBuilder::new("modified".to_string(), SqlOrder::ASC))
            }


            let mut query = conn.prepare(&format!("SELECT id, name, type, alt, params, imdb, slug, tmdb, trakt, tvdb, otherids, year, modified, added, imdb_rating, imdb_votes, trailer, maxCreated, trakt_rating, trakt_votes  FROM series {}{}", where_query.format(), where_query.format_order()))?;
            let rows = query.query_map(
            where_query.values(), Self::row_to_serie,
            )?;
            let backups:Vec<Serie> = rows.collect::<std::result::Result<Vec<Serie>, rusqlite::Error>>()?; 
            Ok(backups)
        }).await?;
        Ok(row)
    }
    pub async fn get_serie(&self, credential_id: &str) -> Result<Option<Serie>> {
        let credential_id = credential_id.to_string();
        let row = self.connection.call( move |conn| { 
            let mut query = conn.prepare("SELECT id, name, type, alt, params, imdb, slug, tmdb, trakt, tvdb, otherids, year, modified, added, imdb_rating, imdb_votes, trailer, maxCreated, trakt_rating, trakt_votes FROM series WHERE id = ?")?;
            let row = query.query_row(
            [credential_id],Self::row_to_serie).optional()?;
            Ok(row)
        }).await?;
        Ok(row)
    }



    pub async fn update_serie(&self, serie_id: &str, update: SerieForUpdate) -> Result<()> {
        let id = serie_id.to_string();
        let existing = self.get_serie(serie_id).await?.ok_or_else( || Error::NotFound)?;
        self.connection.call( move |conn| { 
            let mut where_query = QueryBuilder::new();
            where_query.add_update(update.name.clone(), QueryWhereType::Equal("name".to_string()));
            where_query.add_update(update.kind, QueryWhereType::Equal("type".to_string()));
            where_query.add_update(update.imdb, QueryWhereType::Equal("imdb".to_string()));
            where_query.add_update(update.slug, QueryWhereType::Equal("slug".to_string()));
            where_query.add_update(update.tmdb, QueryWhereType::Equal("tmdb".to_string()));
            where_query.add_update(update.trakt, QueryWhereType::Equal("trakt".to_string()));
            where_query.add_update(update.tvdb, QueryWhereType::Equal("tvdb".to_string()));
            where_query.add_update(update.otherids, QueryWhereType::Equal("otherids".to_string()));
            where_query.add_update(update.imdb_rating, QueryWhereType::Equal("imdb_rating".to_string()));
            where_query.add_update(update.imdb_votes, QueryWhereType::Equal("imdb_votes".to_string()));
            where_query.add_update(update.trakt_rating, QueryWhereType::Equal("trakt_rating".to_string()));
            where_query.add_update(update.trakt_votes, QueryWhereType::Equal("trakt_votes".to_string()));
            where_query.add_update(update.year, QueryWhereType::Equal("year".to_string()));
            where_query.add_update(update.max_created, QueryWhereType::Equal("maxCreated".to_string()));

            let alts = replace_add_remove_from_array(existing.alt, update.alt, update.add_alts, update.remove_alts);
            where_query.add_update(to_pipe_separated_optional(alts), QueryWhereType::Equal("alt".to_string()));






            where_query.add_where(Some(id), QueryWhereType::Equal("id".to_string()));
            

            let update_sql = format!("UPDATE series SET {} {}", where_query.format_update(), where_query.format());

            conn.execute(&update_sql, where_query.values())?;
            Ok(())
        }).await?;

        Ok(())
    }

    pub async fn add_serie(&self, serie: SerieForInsert) -> Result<()> {
        self.connection.call( move |conn| { 

            conn.execute("INSERT INTO series (id, name, type, alt, params, imdb, slug, tmdb, trakt, tvdb, otherids, year, imdb_rating, imdb_votes, trailer, trakt_rating, trakt_votes)
            VALUES (?, ?, ? ,?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)", params![
                serie.id,
                serie.name,
                serie.kind,
                to_pipe_separated_optional(serie.alt),
                serie.params,
                serie.imdb,
                serie.slug,
                serie.tmdb,
                serie.trakt,
                serie.tvdb,
                serie.otherids,
                serie.year,
                serie.imdb_rating,
                serie.imdb_votes,
                serie.trailer,
                serie.trakt_rating,
                serie.trakt_votes
            ])?;
            
            Ok(())
        }).await?;
        Ok(())
    }

    pub async fn remove_serie(&self, serie_id: String) -> Result<()> {
        self.connection.call( move |conn| { 
            conn.execute("DELETE FROM series WHERE id = ?", &[&serie_id])?;
            Ok(())
        }).await?;
        Ok(())
    }
}
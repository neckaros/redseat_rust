use rusqlite::{params, OptionalExtension, Row};

use crate::{domain::{serie::Serie, MediasIds}, model::{series::{SerieForUpdate, SerieQuery}, store::{from_pipe_separated_optional, sql::{OrderBuilder, QueryBuilder, QueryWhereType, RsQueryBuilder, SqlOrder, SqlWhereType}, to_pipe_separated_optional}}, tools::{array_tools::replace_add_remove_from_array, image_tools::ImageType}};
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

            status:  row.get(20)?,

            posterv:  row.get(21)?,
            backgroundv:  row.get(22)?,
            cardv:  row.get(23)?,

        })
    }

    pub async fn get_series(&self, query: SerieQuery) -> Result<Vec<Serie>> {
        let row = self.connection.call( move |conn| { 
            let mut where_query = RsQueryBuilder::new();
            if let Some(q) = query.after {
                where_query.add_where(SqlWhereType::After("modified".to_string(), Box::new(q)));
            }
            if query.after.is_some() {
                where_query.add_oder(OrderBuilder::new("modified".to_string(), SqlOrder::ASC))
            }

            if let Some(q) = query.name {
                let name_queries = vec![SqlWhereType::EqualWithAlt("name".to_owned(), "alt".to_owned(), "|".to_owned(), Box::new(q.clone()))];
                where_query.add_where(SqlWhereType::Or(name_queries));
            }

            where_query.add_oder(OrderBuilder::new(query.sort.to_string(), query.order));


            let mut query = conn.prepare(&format!("SELECT id, name, type, alt, params, imdb, slug, tmdb, trakt, tvdb, otherids, year, modified, added, imdb_rating, imdb_votes, trailer, maxCreated, trakt_rating, trakt_votes, status, posterv, backgroundv, cardv  FROM series {}{}", where_query.format(), where_query.format_order()))?;
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
            let mut query = conn.prepare("SELECT id, name, type, alt, params, imdb, slug, tmdb, trakt, tvdb, otherids, year, modified, added, imdb_rating, imdb_votes, trailer, maxCreated, trakt_rating, trakt_votes, status, posterv, backgroundv, cardv FROM series WHERE id = ?")?;
            let row = query.query_row(
            [credential_id],Self::row_to_serie).optional()?;
            Ok(row)
        }).await?;
        Ok(row)
    }

    pub async fn get_serie_by_external_id(&self, ids: MediasIds) -> Result<Option<Serie>> {
        let row = self.connection.call( move |conn| { 
            let mut query = conn.prepare("SELECT 
            id, name, type, alt, params, imdb, slug, tmdb, trakt, tvdb, otherids, year, modified, added, imdb_rating, imdb_votes, trailer, maxCreated, trakt_rating, trakt_votes, status 
            FROM series 
            WHERE 
            id = ? or imdb = ? or slug = ? or tmdb = ? or trakt = ? or tvdb = ?")?;
            let row = query.query_row(
            params![ids.redseat.unwrap_or("zz".to_string()), ids.imdb.unwrap_or("zz".to_string()), ids.slug.unwrap_or("zz".to_string()), ids.tmdb.unwrap_or(0), ids.trakt.unwrap_or(0), ids.tvdb.unwrap_or(0)],Self::row_to_serie).optional()?;
            Ok(row)
        }).await?;
        Ok(row)
    }

    pub async fn update_serie(&self, serie_id: &str, update: SerieForUpdate) -> Result<()> {
        let id = serie_id.to_string();
        let existing = self.get_serie(serie_id).await?.ok_or_else( || Error::NotFound)?;
        self.connection.call( move |conn| { 
            let mut where_query = QueryBuilder::new();

            where_query.add_update(&update.name, "name");
            where_query.add_update(&update.kind, "type");

            where_query.add_update(&update.status, "status");
            where_query.add_update(&update.trailer, "trailer");
            
            where_query.add_update(&update.imdb, "imdb");
            where_query.add_update(&update.slug, "slug");
            where_query.add_update(&update.tmdb, "tmdb");
            where_query.add_update(&update.trakt, "trakt");
            where_query.add_update(&update.tvdb, "tvdb");
            where_query.add_update(&update.otherids, "otherids");
            where_query.add_update(&update.imdb_rating, "imdb_rating");
            where_query.add_update(&update.imdb_votes, "imdb_votes");
            where_query.add_update(&update.trakt_rating, "trakt_rating");
            where_query.add_update(&update.trakt_votes, "trakt_votes");


            where_query.add_update(&update.year, "year");
            where_query.add_update(&update.max_created, "max_created");

            let alts = replace_add_remove_from_array(existing.alt, update.alt, update.add_alts, update.remove_alts);
            let alts = to_pipe_separated_optional(alts);
            where_query.add_update(&alts, "alt");

            where_query.add_where(QueryWhereType::Equal("id", &id));
            

            let update_sql = format!("UPDATE series SET {} {}", where_query.format_update(), where_query.format());

            conn.execute(&update_sql, where_query.values())?;
            Ok(())
        }).await?;

        Ok(())
    }

    pub async fn update_serie_image(&self, serie_id: String, kind: ImageType) -> Result<()> {

        self.connection.call( move |conn| { 
            match kind {
                ImageType::Poster => conn.execute("update series set posterv = ifnull(posterv, 0) + 1 WHERE id = ?", params![serie_id])?,
                ImageType::Background => conn.execute("update series set backgroundv = ifnull(backgroundv, 0) + 1 WHERE id = ?", params![serie_id])?,
                ImageType::Still => 0,
                ImageType::Card => conn.execute("update series set cardv = ifnull(cardv, 0) + 1 WHERE id = ?", params![serie_id])?,
                ImageType::ClearLogo => 0,
                ImageType::ClearArt => 0,
                ImageType::Custom(_) => 0,
            };

            
            Ok(())
        }).await?;
        Ok(())
    }

    pub async fn add_serie(&self, serie: Serie) -> Result<()> {
        self.connection.call( move |conn| { 

            conn.execute("INSERT INTO series (id, name, type, alt, params, imdb, slug, tmdb, trakt, tvdb, otherids, year, imdb_rating, imdb_votes, trailer, trakt_rating, trakt_votes, status)
            VALUES (?, ?, ? ,?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)", params![
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
                serie.trakt_votes,
                serie.status
            ])?;
            
            Ok(())
        }).await?;
        Ok(())
    }

    pub async fn remove_serie(&self, serie_id: String) -> Result<()> {
        self.connection.call( move |conn| { 
            conn.execute("DELETE FROM series WHERE id = ?", &[&serie_id])?;
            conn.execute("DELETE FROM episodes WHERE serie_ref = ?", &[&serie_id])?;
            conn.execute("DELETE FROM media_serie_mapping WHERE serie_ref = ?", &[&serie_id])?;
            conn.execute("INSERT INTO deleted (id, type) VALUES (?, ?)", &[&serie_id, "serie"])?;


            Ok(())
        }).await?;
        Ok(())
    }
}
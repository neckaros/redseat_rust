use rusqlite::{params, types::{FromSql, FromSqlError, FromSqlResult, ToSqlOutput, ValueRef}, OptionalExtension, Row, ToSql};

use super::{Result, SqliteLibraryStore};
use crate::{domain::{movie::{Movie, MovieForUpdate, MovieStatus}, MediasIds}, model::{movies::MovieQuery, store::{from_pipe_separated_optional, sql::{OrderBuilder, QueryBuilder, QueryWhereType, RsQueryBuilder, SqlOrder, SqlWhereType}, to_pipe_separated_optional}, Error}, tools::{array_tools::replace_add_remove_from_array, clock::now}};

impl FromSql for MovieStatus {
    fn column_result(value: ValueRef) -> FromSqlResult<Self> {
        String::column_result(value).and_then(|as_string| {
            MovieStatus::try_from(&*as_string).map_err(|_| FromSqlError::InvalidType)
        })
    }
}

impl ToSql for MovieStatus {
    fn to_sql(&self) -> rusqlite::Result<rusqlite::types::ToSqlOutput<'_>> {
        Ok(ToSqlOutput::from(self.to_string()))
    }
}



impl SqliteLibraryStore {
  
    fn row_to_movie(row: &Row) -> rusqlite::Result<Movie> {
        Ok(Movie {
            id: row.get(0)?,
            name: row.get(1)?,
            kind: row.get(2)?,
            year: row.get(3)?,
            airdate: row.get(4)?,
            digitalairdate: row.get(5)?,


            duration: row.get(6)?,
            overview: row.get(7)?,
            country: row.get(8)?,
            status: row.get(9)?,

            lang: row.get(10)?,
            original: row.get(11)?,

            imdb: row.get(12)?,
            slug: row.get(13)?,
            tmdb: row.get(14)?,
            trakt: row.get(15)?,
            otherids: row.get(16)?,

            imdb_rating: row.get(17)?,
            imdb_votes: row.get(18)?,
            trakt_rating: row.get(19)?,
            trakt_votes: row.get(20)?,
            
            trailer: row.get(21)?,

            modified: row.get(22)?,
            added: row.get(23)?,

            ..Default::default()

        })
    }

    pub async fn get_movies(&self, query: MovieQuery) -> Result<Vec<Movie>> {
        let row = self.connection.call( move |conn| { 
            let mut where_query = RsQueryBuilder::new();
            if let Some(q) = query.after {
                where_query.add_where(SqlWhereType::After("modified".to_owned(), Box::new(q)));
            }

            if let Some(in_digital) = query.in_digital {
                let now = now().timestamp_millis();
                if in_digital {
                    where_query.add_where(SqlWhereType::Before("digitalairdate".to_owned(), Box::new(now)))
                } else {
                    where_query.add_where(SqlWhereType::After("digitalairdate".to_owned(), Box::new(now)))
                }
            }


            where_query.add_oder(OrderBuilder { column: query.sort.to_string(), order: query.order.unwrap_or(SqlOrder::ASC) });



            let mut query = conn.prepare(&format!("SELECT 
            id, name, type, year, airdate, digitalairdate, 
            duration, overview, country, status,
            lang, original,
            imdb, slug, tmdb, trakt, otherids, 
            imdb_rating, imdb_votes, trakt_rating, trakt_votes, trailer,
            modified, added FROM movies {}{}", where_query.format(), where_query.format_order()))?;
            let rows = query.query_map(
            where_query.values(), Self::row_to_movie,
            )?;
            let backups:Vec<Movie> = rows.collect::<std::result::Result<Vec<Movie>, rusqlite::Error>>()?; 
            Ok(backups)
        }).await?;
        Ok(row)
    }
    pub async fn get_movie(&self, credential_id: &str) -> Result<Option<Movie>> {
        let credential_id = credential_id.to_string();
        let row = self.connection.call( move |conn| { 
            let mut query = conn.prepare("SELECT 
            id, name, type, year, airdate, digitalairdate, 
            duration, overview, country, status,
            lang, original,
            imdb, slug, tmdb, trakt, otherids, 
            imdb_rating, imdb_votes, trakt_rating, trakt_votes, trailer,
            modified, added FROM movies WHERE id = ?")?;
            let row = query.query_row(
            [credential_id],Self::row_to_movie).optional()?;
            Ok(row)
        }).await?;
        Ok(row)
    }

    pub async fn get_movie_by_external_id(&self, ids: MediasIds) -> Result<Option<Movie>> {
        
        //println!("{}, {}, {}, {}, {}",i.imdb.unwrap_or("zz".to_string()), i.slug.unwrap_or("zz".to_string()), i.tmdb.unwrap_or(0), i.trakt.unwrap_or(0), i.tvdb.unwrap_or(0));
        let row = self.connection.call( move |conn| { 
            let mut query = conn.prepare("SELECT  
            id, name, type, year, airdate, digitalairdate, 
            duration, overview, country, status,
            lang, original,
            imdb, slug, tmdb, trakt, otherids, 
            imdb_rating, imdb_votes, trakt_rating, trakt_votes, trailer,
            modified, added FROM movies 
            WHERE 
            imdb = ? or slug = ? or tmdb = ? or trakt = ?")?;
            let row = query.query_row(
            params![ids.imdb.unwrap_or("zz".to_string()), ids.slug.unwrap_or("zz".to_string()), ids.tmdb.unwrap_or(0), ids.trakt.unwrap_or(0)],Self::row_to_movie).optional()?;
            Ok(row)
        }).await?;
        Ok(row)
    }

    pub async fn update_movie(&self, movie_id: &str, update: MovieForUpdate) -> Result<()> {
        let id = movie_id.to_string();
        //let existing = self.get_movie(movie_id).await?.ok_or_else( || Error::NotFound)?;
        self.connection.call( move |conn| { 
            let mut where_query = QueryBuilder::new();

            where_query.add_update(&update.name, "name");
            where_query.add_update(&update.kind, "type");

            where_query.add_update(&update.airdate, "airdate");
            where_query.add_update(&update.digitalairdate, "digitalairdate");

            where_query.add_update(&update.status, "status");
            where_query.add_update(&update.trailer, "trailer");
            
            where_query.add_update(&update.imdb, "imdb");
            where_query.add_update(&update.slug, "slug");
            where_query.add_update(&update.tmdb, "tmdb");
            where_query.add_update(&update.trakt, "trakt");
            where_query.add_update(&update.otherids, "otherids");
            
            where_query.add_update(&update.imdb_rating, "imdb_rating");
            where_query.add_update(&update.imdb_votes, "imdb_votes");
            where_query.add_update(&update.trakt_rating, "trakt_rating");
            where_query.add_update(&update.trakt_votes, "trakt_votes");


            where_query.add_update(&update.year, "year");

            where_query.add_where(QueryWhereType::Equal("id", &id));
            

            let update_sql = format!("UPDATE movies SET {} {}", where_query.format_update(), where_query.format());

            conn.execute(&update_sql, where_query.values())?;
            Ok(())
        }).await?;

        Ok(())
    }

    pub async fn add_movie(&self, movie: Movie) -> Result<()> {
        self.connection.call( move |conn| { 

            conn.execute("INSERT INTO movies ( 
                id, name, type, year, airdate, digitalairdate, 
                duration, overview, country, status,
                lang, original,
                imdb, slug, tmdb, trakt, otherids, 
                imdb_rating, imdb_votes, trakt_rating, trakt_votes, trailer)
            VALUES (?, ?, ? ,?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)", params![
                movie.id,
                movie.name,
                movie.kind,
                movie.year,
                movie.airdate,
                movie.digitalairdate,

                movie.duration, movie.overview, movie.country, movie.status,
                movie.lang, movie.original,
                movie.imdb, movie.slug, movie.tmdb, movie.trakt, movie.otherids,

               
                movie.imdb_rating,
                movie.imdb_votes,
                movie.trakt_rating,
                movie.trakt_votes,

                movie.trailer,
            ])?;
            
            Ok(())
        }).await?;
        Ok(())
    }

    pub async fn remove_movie(&self, movie_id: String) -> Result<()> {
        self.connection.call( move |conn| { 
            conn.execute("DELETE FROM movies WHERE id = ?", [&movie_id])?;
            Ok(())
        }).await?;
        Ok(())
    }
}
use rs_plugin_common_interfaces::{domain::rs_ids::RsIds, ImageType};
use rusqlite::{
    params,
    types::{FromSql, FromSqlError, FromSqlResult, ToSqlOutput, ValueRef},
    OptionalExtension, Row, ToSql,
};

use super::{Result, SqliteLibraryStore};
use crate::{
    domain::movie::{Movie, MovieForUpdate, MovieStatus},
    model::{
        movies::MovieQuery,
        store::{
            from_pipe_separated_optional,
            sql::{
                pagination_clause, OrderBuilder, QueryBuilder, QueryWhereType, RsQueryBuilder,
                SqlOrder, SqlWhereType,
            },
            to_pipe_separated_optional,
        },
        Error,
    },
    tools::{array_tools::replace_add_remove_from_array, clock::now},
};

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

            posterv: row.get(24)?,
            backgroundv: row.get(25)?,
            cardv: row.get(26)?,

            ..Default::default()
        })
    }

    pub async fn get_movies(&self, query: MovieQuery) -> Result<Vec<Movie>> {
        let pagination = pagination_clause(query.limit, query.offset)?;
        let row = self
            .connection
            .call(move |conn| {
                let mut where_query = RsQueryBuilder::new();
                if let Some(q) = query.after {
                    where_query.add_where(SqlWhereType::After("modified".to_owned(), Box::new(q)));
                }

                if let Some(in_digital) = query.in_digital {
                    let now = now().timestamp_millis();
                    if in_digital {
                        where_query.add_where(SqlWhereType::Before(
                            "digitalairdate".to_owned(),
                            Box::new(now),
                        ))
                    } else {
                        where_query.add_where(SqlWhereType::After(
                            "digitalairdate".to_owned(),
                            Box::new(now),
                        ))
                    }
                }

                where_query.add_oder(OrderBuilder {
                    column: query.sort.to_string(),
                    order: query.order.unwrap_or(SqlOrder::ASC),
                });
                where_query.add_oder(OrderBuilder::new("id".to_string(), SqlOrder::ASC));

                let mut query = conn.prepare(&format!(
                    "SELECT 
            id, name, type, year, airdate, digitalairdate, 
            duration, overview, country, status,
            lang, original,
            imdb, slug, tmdb, trakt, otherids, 
            imdb_rating, imdb_votes, trakt_rating, trakt_votes, trailer,
            modified, added, posterv, backgroundv, cardv FROM movies {}{}{}",
                    where_query.format(),
                    where_query.format_order(),
                    pagination
                ))?;
                let rows = query.query_map(where_query.values(), Self::row_to_movie)?;
                let backups: Vec<Movie> =
                    rows.collect::<std::result::Result<Vec<Movie>, rusqlite::Error>>()?;
                Ok(backups)
            })
            .await?;
        Ok(row)
    }
    pub async fn get_movie(&self, credential_id: &str) -> Result<Option<Movie>> {
        let credential_id = credential_id.to_string();
        let row = self
            .connection
            .call(move |conn| {
                let mut query = conn.prepare(
                    "SELECT 
            id, name, type, year, airdate, digitalairdate, 
            duration, overview, country, status,
            lang, original,
            imdb, slug, tmdb, trakt, otherids, 
            imdb_rating, imdb_votes, trakt_rating, trakt_votes, trailer,
            modified, added, posterv, backgroundv, cardv FROM movies WHERE id = ?",
                )?;
                let row = query
                    .query_row([credential_id], Self::row_to_movie)
                    .optional()?;
                Ok(row)
            })
            .await?;
        Ok(row)
    }

    pub async fn get_movie_by_external_id(&self, ids: RsIds) -> Result<Option<Movie>> {
        //println!("{}, {}, {}, {}, {}",i.imdb.unwrap_or("zz".to_string()), i.slug.unwrap_or("zz".to_string()), i.tmdb.unwrap_or(0), i.trakt.unwrap_or(0), i.tvdb.unwrap_or(0));
        let row = self
            .connection
            .call(move |conn| {
                let mut query = conn.prepare(
                    "SELECT  
            id, name, type, year, airdate, digitalairdate, 
            duration, overview, country, status,
            lang, original,
            imdb, slug, tmdb, trakt, otherids, 
            imdb_rating, imdb_votes, trakt_rating, trakt_votes, trailer,
            modified, added, posterv, backgroundv, cardv FROM movies 
            WHERE 
            imdb = ? or slug = ? or tmdb = ? or trakt = ?",
                )?;
                let row = query
                    .query_row(
                        params![
                            ids.imdb().unwrap_or("zz").to_string(),
                            ids.slug().unwrap_or("zz").to_string(),
                            ids.tmdb().unwrap_or(0),
                            ids.trakt().unwrap_or(0)
                        ],
                        Self::row_to_movie,
                    )
                    .optional()?;
                Ok(row)
            })
            .await?;
        Ok(row)
    }

    pub async fn update_movie(&self, movie_id: &str, update: MovieForUpdate) -> Result<()> {
        let id = movie_id.to_string();
        //let existing = self.get_movie(movie_id).await?.ok_or_else( || Error::NotFound)?;
        self.connection
            .call(move |conn| {
                let mut where_query = QueryBuilder::new();

                where_query.add_update(&update.name, "name");
                where_query.add_update(&update.kind, "type");

                where_query.add_update(&update.airdate, "airdate");
                where_query.add_update(&update.digitalairdate, "digitalairdate");

                where_query.add_update(&update.duration, "duration");
                where_query.add_update(&update.overview, "overview");
                where_query.add_update(&update.country, "country");
                where_query.add_update(&update.lang, "lang");
                where_query.add_update(&update.original, "original");

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

                let update_sql = format!(
                    "UPDATE movies SET {} {}",
                    where_query.format_update(),
                    where_query.format()
                );

                conn.execute(&update_sql, where_query.values())?;
                Ok(())
            })
            .await?;

        Ok(())
    }

    pub async fn update_movie_image(&self, movie_id: String, kind: ImageType) -> Result<()> {
        self.connection
            .call(move |conn| {
                match kind {
                    ImageType::Poster => conn.execute(
                        "update movies set posterv = ifnull(posterv, 0) + 1 WHERE id = ?",
                        params![movie_id],
                    )?,
                    ImageType::Background => conn.execute(
                        "update movies set backgroundv = ifnull(backgroundv, 0) + 1 WHERE id = ?",
                        params![movie_id],
                    )?,
                    ImageType::Still => 0,
                    ImageType::Card => conn.execute(
                        "update movies set cardv = ifnull(cardv, 0) + 1 WHERE id = ?",
                        params![movie_id],
                    )?,
                    ImageType::ClearLogo => 0,
                    ImageType::ClearArt => 0,
                    ImageType::Custom(_) => 0,
                };

                Ok(())
            })
            .await?;
        Ok(())
    }

    pub async fn add_movie(&self, movie: Movie) -> Result<()> {
        self.connection
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO movies ( 
                id, name, type, year, airdate, digitalairdate, 
                duration, overview, country, status,
                lang, original,
                imdb, slug, tmdb, trakt, otherids, 
                imdb_rating, imdb_votes, trakt_rating, trakt_votes, trailer)
            VALUES (?, ?, ? ,?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                    params![
                        movie.id,
                        movie.name,
                        movie.kind,
                        movie.year,
                        movie.airdate,
                        movie.digitalairdate,
                        movie.duration,
                        movie.overview,
                        movie.country,
                        movie.status,
                        movie.lang,
                        movie.original,
                        movie.imdb,
                        movie.slug,
                        movie.tmdb,
                        movie.trakt,
                        movie.otherids,
                        movie.imdb_rating,
                        movie.imdb_votes,
                        movie.trakt_rating,
                        movie.trakt_votes,
                        movie.trailer,
                    ],
                )?;

                Ok(())
            })
            .await?;
        Ok(())
    }

    pub async fn remove_movie(&self, movie_id: String) -> Result<()> {
        self.connection
            .call(move |conn| {
                conn.execute("DELETE FROM movies WHERE id = ?", [&movie_id])?;
                Ok(())
            })
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::SqliteLibraryStore;
    use crate::domain::movie::{Movie, MovieForUpdate};

    #[tokio::test]
    async fn movie_metadata_updates_roundtrip() {
        let connection = tokio_rusqlite::Connection::open_in_memory().await.unwrap();
        let store = SqliteLibraryStore::new(connection).await.unwrap();
        store
            .add_movie(Movie {
                id: "movie-refresh".to_string(),
                name: "Example".to_string(),
                imdb: Some("tt1234567".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();

        // Each field must trigger an update even when it is the only change.
        for update in [
            MovieForUpdate {
                overview: Some("A movie overview".to_string()),
                ..Default::default()
            },
            MovieForUpdate {
                duration: Some(120),
                ..Default::default()
            },
            MovieForUpdate {
                country: Some("US".to_string()),
                ..Default::default()
            },
            MovieForUpdate {
                lang: Some("en".to_string()),
                ..Default::default()
            },
            MovieForUpdate {
                original: Some("Original title".to_string()),
                ..Default::default()
            },
            MovieForUpdate {
                imdb_rating: Some(7.5),
                imdb_votes: Some(1234),
                ..Default::default()
            },
        ] {
            assert!(update.has_update());
            store.update_movie("movie-refresh", update).await.unwrap();
        }

        let movie = store.get_movie("movie-refresh").await.unwrap().unwrap();
        assert_eq!(movie.overview.as_deref(), Some("A movie overview"));
        assert_eq!(movie.duration, Some(120));
        assert_eq!(movie.country.as_deref(), Some("US"));
        assert_eq!(movie.lang.as_deref(), Some("en"));
        assert_eq!(movie.original.as_deref(), Some("Original title"));
        assert_eq!(movie.imdb_rating, Some(7.5));
        assert_eq!(movie.imdb_votes, Some(1234));
        assert_eq!(movie.imdb.as_deref(), Some("tt1234567"));
        assert_eq!(movie.name, "Example");
        assert!(!MovieForUpdate::default().has_update());
    }
    #[tokio::test]
    async fn movie_refresh_other_ids_and_max_duration_roundtrip() {
        use rs_plugin_common_interfaces::domain::other_ids::OtherIds;
        let connection = tokio_rusqlite::Connection::open_in_memory().await.unwrap();
        let store = SqliteLibraryStore::new(connection).await.unwrap();
        let mut movie = Movie {
            id: "refresh-ids".to_string(),
            name: "Example".to_string(),
            ..Default::default()
        };
        store.add_movie(movie.clone()).await.unwrap();
        for value in ["provider:first", "provider:changed"] {
            let refreshed = Movie {
                otherids: Some(OtherIds::from(vec![value.to_string()])),
                duration: Some(u32::MAX),
                ..movie.clone()
            };
            let update = MovieForUpdate::from_refresh(&movie, refreshed.clone()).unwrap();
            store.update_movie(&movie.id, update).await.unwrap();
            movie = store.get_movie(&movie.id).await.unwrap().unwrap();
            assert_eq!(movie.otherids, refreshed.otherids);
            assert_eq!(movie.duration, Some(u32::MAX));
        }
    }
}

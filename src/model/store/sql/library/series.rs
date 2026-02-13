use rs_plugin_common_interfaces::{domain::rs_ids::RsIds, ImageType};
use rusqlite::{params, OptionalExtension, Row};

use super::{Result, SqliteLibraryStore};
use crate::model::Error;
use crate::{
    domain::serie::Serie,
    model::{
        series::{SerieForUpdate, SerieQuery},
        store::{
            from_pipe_separated_optional,
            sql::{
                OrderBuilder, QueryBuilder, QueryWhereType, RsQueryBuilder, SqlOrder, SqlWhereType,
            },
            to_pipe_separated_optional,
        },
    },
    plugins::sources::error::SourcesError,
    tools::array_tools::replace_add_remove_from_array,
};

const SERIE_SQL_FIELDS: &str = "id, name, type, alt, params, imdb, slug, tmdb, trakt, tvdb, otherids, openlibrary_work_id, anilist_manga_id, mangadex_manga_uuid, myanimelist_manga_id, year, modified, added, imdb_rating, imdb_votes, trailer, maxCreated, trakt_rating, trakt_votes, status, posterv, backgroundv, cardv";

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
            openlibrary_work_id: row.get(11)?,
            anilist_manga_id: row.get(12)?,
            mangadex_manga_uuid: row.get(13)?,
            myanimelist_manga_id: row.get(14)?,
            year: row.get(15)?,
            modified: row.get(16)?,
            added: row.get(17)?,

            imdb_rating: row.get(18)?,
            imdb_votes: row.get(19)?,

            trailer: row.get(20)?,
            max_created: row.get(21)?,
            trakt_rating: row.get(22)?,
            trakt_votes: row.get(23)?,

            status: row.get(24)?,

            posterv: row.get(25)?,
            backgroundv: row.get(26)?,
            cardv: row.get(27)?,
        })
    }

    pub async fn get_series(&self, query: SerieQuery) -> Result<Vec<Serie>> {
        let row = self
            .connection
            .call(move |conn| {
                let mut where_query = RsQueryBuilder::new();
                if let Some(q) = query.after {
                    where_query.add_where(SqlWhereType::After("modified".to_string(), Box::new(q)));
                }
                if query.after.is_some() {
                    where_query.add_oder(OrderBuilder::new("modified".to_string(), SqlOrder::ASC))
                }

                if let Some(q) = query.name {
                    let name_queries = vec![SqlWhereType::EqualWithAlt(
                        "name".to_owned(),
                        "alt".to_owned(),
                        "|".to_owned(),
                        Box::new(q.clone()),
                    )];
                    where_query.add_where(SqlWhereType::Or(name_queries));
                }

                where_query.add_oder(OrderBuilder::new(query.sort.to_string(), query.order));

                let mut query = conn.prepare(&format!(
                    "SELECT {}  FROM series {}{}",
                    SERIE_SQL_FIELDS,
                    where_query.format(),
                    where_query.format_order()
                ))?;
                let rows = query.query_map(where_query.values(), Self::row_to_serie)?;
                let backups: Vec<Serie> =
                    rows.collect::<std::result::Result<Vec<Serie>, rusqlite::Error>>()?;
                Ok(backups)
            })
            .await?;
        Ok(row)
    }
    pub async fn get_serie(&self, credential_id: &str) -> Result<Option<Serie>> {
        let credential_id = credential_id.to_string();
        let row = self
            .connection
            .call(move |conn| {
                let mut query = conn.prepare(&format!(
                    "SELECT {} FROM series WHERE id = ?",
                    SERIE_SQL_FIELDS
                ))?;
                let row = query
                    .query_row([credential_id], Self::row_to_serie)
                    .optional()?;
                Ok(row)
            })
            .await?;
        Ok(row)
    }

    pub async fn get_serie_by_external_id(&self, ids: RsIds) -> Result<Option<Serie>> {
        let row = self.connection.call( move |conn| { 
            let mut query = conn.prepare(&format!("SELECT 
            {} 
            FROM series 
            WHERE 
            id = ? or imdb = ? or slug = ? or tmdb = ? or trakt = ? or tvdb = ? or openlibrary_work_id = ? or anilist_manga_id = ? or mangadex_manga_uuid = ? or myanimelist_manga_id = ?", SERIE_SQL_FIELDS))?;
            let row = query.query_row(
            params![
                ids.redseat.unwrap_or("zz".to_string()),
                ids.imdb.unwrap_or("zz".to_string()),
                ids.slug.unwrap_or("zz".to_string()),
                ids.tmdb.unwrap_or(0),
                ids.trakt.unwrap_or(0),
                ids.tvdb.unwrap_or(0),
                ids.openlibrary_work_id.unwrap_or("zz".to_string()),
                ids.anilist_manga_id.unwrap_or(0),
                ids.mangadex_manga_uuid.unwrap_or("zz".to_string()),
                ids.myanimelist_manga_id.unwrap_or(0)
            ],Self::row_to_serie).optional()?;
            Ok(row)
        }).await?;
        Ok(row)
    }

    pub async fn update_serie(&self, serie_id: &str, update: SerieForUpdate) -> Result<()> {
        let id = serie_id.to_string();
        let existing = self.get_serie(serie_id).await?.ok_or_else(|| {
            SourcesError::UnableToFindSerie(
                "store".to_string(),
                serie_id.to_string(),
                "update_serie".to_string(),
            )
        })?;
        self.connection
            .call(move |conn| {
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
                where_query.add_update(&update.openlibrary_work_id, "openlibrary_work_id");
                where_query.add_update(&update.anilist_manga_id, "anilist_manga_id");
                where_query.add_update(&update.mangadex_manga_uuid, "mangadex_manga_uuid");
                where_query.add_update(&update.myanimelist_manga_id, "myanimelist_manga_id");
                where_query.add_update(&update.imdb_rating, "imdb_rating");
                where_query.add_update(&update.imdb_votes, "imdb_votes");
                where_query.add_update(&update.trakt_rating, "trakt_rating");
                where_query.add_update(&update.trakt_votes, "trakt_votes");

                where_query.add_update(&update.year, "year");
                where_query.add_update(&update.max_created, "max_created");

                let alts = replace_add_remove_from_array(
                    existing.alt,
                    update.alt,
                    update.add_alts,
                    update.remove_alts,
                );
                let alts = to_pipe_separated_optional(alts);
                where_query.add_update(&alts, "alt");

                where_query.add_where(QueryWhereType::Equal("id", &id));

                let update_sql = format!(
                    "UPDATE series SET {} {}",
                    where_query.format_update(),
                    where_query.format()
                );

                conn.execute(&update_sql, where_query.values())?;
                Ok(())
            })
            .await?;

        Ok(())
    }

    pub async fn update_serie_image(&self, serie_id: String, kind: ImageType) -> Result<()> {
        self.connection
            .call(move |conn| {
                match kind {
                    ImageType::Poster => conn.execute(
                        "update series set posterv = ifnull(posterv, 0) + 1 WHERE id = ?",
                        params![serie_id],
                    )?,
                    ImageType::Background => conn.execute(
                        "update series set backgroundv = ifnull(backgroundv, 0) + 1 WHERE id = ?",
                        params![serie_id],
                    )?,
                    ImageType::Still => 0,
                    ImageType::Card => conn.execute(
                        "update series set cardv = ifnull(cardv, 0) + 1 WHERE id = ?",
                        params![serie_id],
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

    pub async fn add_serie(&self, serie: Serie) -> Result<()> {
        self.connection.call( move |conn| { 

            conn.execute("INSERT INTO series (id, name, type, alt, params, imdb, slug, tmdb, trakt, tvdb, otherids, openlibrary_work_id, anilist_manga_id, mangadex_manga_uuid, myanimelist_manga_id, year, imdb_rating, imdb_votes, trailer, trakt_rating, trakt_votes, status)
            VALUES (?, ?, ? ,?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)", params![
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
                serie.openlibrary_work_id,
                serie.anilist_manga_id,
                serie.mangadex_manga_uuid,
                serie.myanimelist_manga_id,
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
        self.connection
            .call(move |conn| {
                conn.execute("DELETE FROM series WHERE id = ?", &[&serie_id])?;
                conn.execute("DELETE FROM episodes WHERE serie_ref = ?", &[&serie_id])?;
                conn.execute(
                    "DELETE FROM media_serie_mapping WHERE serie_ref = ?",
                    &[&serie_id],
                )?;
                conn.execute(
                    "INSERT INTO deleted (id, type) VALUES (?, ?)",
                    &[&serie_id, "serie"],
                )?;

                Ok(())
            })
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::SqliteLibraryStore;
    use crate::{domain::serie::Serie, model::series::SerieForUpdate};

    #[tokio::test]
    async fn series_book_identity_fields_roundtrip() {
        let connection = tokio_rusqlite::Connection::open_in_memory().await.unwrap();
        let store = SqliteLibraryStore::new(connection).await.unwrap();
        let serie = Serie {
            id: "serie-books-ids".to_string(),
            name: "One Piece".to_string(),
            anilist_manga_id: Some(30013),
            mangadex_manga_uuid: Some("manga-level-uuid".to_string()),
            myanimelist_manga_id: Some(13),
            openlibrary_work_id: Some("OL123W".to_string()),
            ..Default::default()
        };
        store.add_serie(serie).await.unwrap();
        let inserted = store.get_serie("serie-books-ids").await.unwrap().unwrap();
        assert_eq!(inserted.anilist_manga_id, Some(30013));
        assert_eq!(
            inserted.mangadex_manga_uuid.as_deref(),
            Some("manga-level-uuid")
        );
        assert_eq!(inserted.myanimelist_manga_id, Some(13));
        assert_eq!(inserted.openlibrary_work_id.as_deref(), Some("OL123W"));

        store
            .update_serie(
                "serie-books-ids",
                SerieForUpdate {
                    myanimelist_manga_id: Some(99),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let updated = store.get_serie("serie-books-ids").await.unwrap().unwrap();
        assert_eq!(updated.myanimelist_manga_id, Some(99));
    }
}

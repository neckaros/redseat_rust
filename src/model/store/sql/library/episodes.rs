use std::collections::{HashMap, HashSet};

use rusqlite::{params, OptionalExtension, Row, ToSql};

use super::{Result, SqliteLibraryStore};
use crate::model::Error;
use crate::{
    domain::{
        episode::{self, Episode, EpisodeWithAction, EpisodeWithShow},
        ElementAction,
    },
    model::{
        episodes::{EpisodeForUpdate, EpisodeQuery, EpisodeSyncResult},
        store::{
            from_pipe_separated_optional,
            sql::{OrderBuilder, QueryBuilder, QueryWhereType, SqlOrder},
            to_pipe_separated_optional,
        },
    },
    plugins::sources::error::SourcesError,
    tools::array_tools::replace_add_remove_from_array,
};

impl SqliteLibraryStore {
    fn row_to_episode(row: &Row) -> rusqlite::Result<Episode> {
        Ok(Episode {
            serie: row.get(0)?,
            season: row.get(1)?,
            number: row.get(2)?,
            abs: row.get(3)?,

            name: row.get(4)?,
            overview: row.get(5)?,

            airdate: row.get(6)?,
            duration: row.get(7)?,

            alt: from_pipe_separated_optional(row.get(8)?),
            params: row.get(9)?,

            imdb: row.get(10)?,
            slug: row.get(11)?,
            tmdb: row.get(12)?,
            trakt: row.get(13)?,
            tvdb: row.get(14)?,
            otherids: row.get(15)?,

            modified: row.get(16)?,
            added: row.get(17)?,

            imdb_rating: row.get(18)?,
            imdb_votes: row.get(19)?,

            trakt_rating: row.get(20)?,
            trakt_votes: row.get(21)?,

            serie_name: row.get(22)?,

            ..Default::default()
        })
    }

    fn row_to_show_episode(row: &Row) -> rusqlite::Result<EpisodeWithShow> {
        Ok(EpisodeWithShow {
            name: row.get(22)?,
            episode: SqliteLibraryStore::row_to_episode(&row)?,
        })
    }

    pub async fn get_episodes(&self, query: EpisodeQuery) -> Result<Vec<Episode>> {
        let row = self
            .connection
            .call(move |conn| {
                let pagination = query.limit.map(|limit| format!(" LIMIT {} OFFSET {}", limit.min(5000), query.offset.unwrap_or(0))).unwrap_or_default();
                let mut where_query = QueryBuilder::new();
                if let Some(q) = &query.after {
                    if q > &0 {
                        where_query.add_where(QueryWhereType::After("u.modified", q));
                    }
                }

                if let Some(q) = &query.aired_after {
                    where_query.add_where(QueryWhereType::After("u.airdate", q));
                }
                if let Some(q) = &query.aired_before {
                    where_query.add_where(QueryWhereType::Before("u.airdate", q));
                }

                if let Some(q) = &query.serie_ref {
                    where_query.add_where(QueryWhereType::Equal("u.serie_ref", q));
                }
                if let Some(q) = &query.season {
                    where_query.add_where(QueryWhereType::Equal("u.season", q));
                }

                if query.not_seasons.len() > 0 {
                    let refed = query
                        .not_seasons
                        .iter()
                        .map(|t| t as &dyn ToSql)
                        .collect::<Vec<_>>();
                    where_query.add_where(QueryWhereType::NotIn("u.season", refed));
                }

                for sorts in query.sorts {
                    where_query.add_oder(OrderBuilder::new(sorts.sort.to_string(), sorts.order));
                }
                where_query.add_oder(OrderBuilder::new("serie_ref".to_string(), SqlOrder::ASC));
                where_query.add_oder(OrderBuilder::new("season".to_string(), SqlOrder::ASC));
                where_query.add_oder(OrderBuilder::new("number".to_string(), SqlOrder::ASC));

                //where_query.add_oder(OrderBuilder::new("season".to_string(), SqlOrder::ASC));
                //where_query.add_oder(OrderBuilder::new("number".to_string(), SqlOrder::ASC));

                let mut query = conn.prepare(&format!(
                    "
SELECT * FROM (
SELECT 
    e.serie_ref,
    COALESCE(e.season, 0) AS season,
    COALESCE(e.number, 0) AS number,
    e.abs,
    e.name,
    e.overview,
    e.airdate,
    e.duration,
    e.alt,
    e.params,
    e.imdb,
    e.slug,
    e.tmdb,
    e.trakt,
    e.tvdb,
    e.otherids,
    COALESCE(e.modified, 0) as modified,
    COALESCE(e.added, 0) as added,
    e.imdb_rating,
    e.imdb_votes,
    e.trakt_rating,
    e.trakt_votes, 
    null as serie_name
FROM 
    episodes e
	
	UNION


SELECT 
    msm.serie_ref,
    COALESCE(msm.season, 0) AS season,
    COALESCE(msm.episode, 0) AS number,
    e.abs,
    e.name,
    e.overview,
    e.airdate,
    e.duration,
    e.alt,
    e.params,
    e.imdb,
    e.slug,
    e.tmdb,
    e.trakt,
    e.tvdb,
    e.otherids,
    COALESCE(e.modified, 0) as modified,
    COALESCE(e.added, 0) as added,
    e.imdb_rating,
    e.imdb_votes,
    e.trakt_rating,
    e.trakt_votes, 
    null as serie_name
FROM 
    media_serie_mapping msm
LEFT JOIN 
    episodes e
ON 
    msm.serie_ref = e.serie_ref 
    AND msm.season = e.season 
    AND msm.episode = e.number
    {}
    )
    as u
    {}{}",
                    where_query.format_order(),
                    where_query.format(),
                    pagination
                ))?;
                //println!("query {:?}", query.expanded_sql());
                let rows = query.query_map(where_query.values(), Self::row_to_episode)?;
                let backups: Vec<Episode> =
                    rows.collect::<std::result::Result<Vec<Episode>, rusqlite::Error>>()?;
                Ok(backups)
            })
            .await?;
        Ok(row)
    }

    pub async fn get_episodes_upcoming(&self, query: EpisodeQuery) -> Result<Vec<Episode>> {
        let row = self
            .connection
            .call(move |conn| {
                let mut stm = conn.prepare(
                    " 
            SELECT * FROM (
    SELECT 
        ep.serie_ref, ep.season, ep.number, ep.abs, ep.name, ep.overview, ep.airdate, ep.duration, 
        ep.alt, ep.params, ep.imdb, ep.slug, ep.tmdb, ep.trakt, ep.tvdb, ep.otherids, 
        ep.modified, ep.added, ep.imdb_rating, ep.imdb_votes, ep.trakt_rating, ep.trakt_votes,
        series.name,
        ROW_NUMBER() OVER (PARTITION BY ep.serie_ref ORDER BY ep.airdate ASC) as rn
    FROM 
        episodes as ep
    LEFT JOIN series ON ep.serie_ref = series.id
    WHERE 
        season <> 0 and
        airdate > round((julianday('now') - 2440587.5)*86400.0 * 1000)
) ranked
WHERE rn = 1
ORDER BY airdate ASC
            LIMIT ?
            ",
                )?;
                let rows = stm.query_map(&[&query.limit.unwrap_or(100)], Self::row_to_episode)?;
                let backups: Vec<Episode> =
                    rows.collect::<std::result::Result<Vec<Episode>, rusqlite::Error>>()?;
                Ok(backups)
            })
            .await?;
        Ok(row)
    }

    pub async fn get_episodes_aired(&self, _query: EpisodeQuery) -> Result<Vec<Episode>> {
        let row = self.connection.call( move |conn| { 
            let mut stm = conn.prepare("SELECT 
            ep.serie_ref, ep.season, ep.number, ep.abs, ep.name, ep.overview, ep.airdate, ep.duration, ep.alt, ep.params, ep.imdb, ep.slug, ep.tmdb, ep.trakt, ep.tvdb, ep.otherids, ep.modified, ep.added, ep.imdb_rating, ep.imdb_votes, ep.trakt_rating, ep.trakt_votes,
            series.name  
            FROM 
            episodes as ep
            LEFT JOIN series ON ep.serie_ref = series.id
            WHERE 
            season <> 0 and
            airdate < round((julianday('now') - 2440587.5)*86400.0 * 1000)
            ORDER BY  ep.airdate ASC,  ep.season ASC,  ep.number
            ")?;
            let rows = stm.query_map(
            params![], Self::row_to_episode,
            )?;
            let backups:Vec<Episode> = rows.collect::<std::result::Result<Vec<Episode>, rusqlite::Error>>()?; 
            Ok(backups)
        }).await?;
        Ok(row)
    }

    pub async fn get_episode(
        &self,
        serie_id: &str,
        season: u32,
        number: u32,
    ) -> Result<Option<Episode>> {
        let serie_id = serie_id.to_string();
        let row = self.connection.call( move |conn| { 
            let mut query = conn.prepare("SELECT serie_ref, season, number, abs, name, overview, airdate, duration, alt, params, imdb, slug, tmdb, trakt, tvdb, otherids, modified, added, imdb_rating, imdb_votes, trakt_rating, trakt_votes, null as serie_name FROM episodes WHERE serie_ref = ? and season = ? and number = ?")?;
            let row = query.query_row(
            params![serie_id, season, number],Self::row_to_episode).optional()?;
            Ok(row)
        }).await?;
        Ok(row)
    }

    pub async fn update_episode(
        &self,
        serie_id: &str,
        season: u32,
        number: u32,
        update: EpisodeForUpdate,
    ) -> Result<()> {
        let id = serie_id.to_string();
        let existing = self
            .get_episode(serie_id, season, number)
            .await?
            .ok_or_else(|| {
                SourcesError::UnableToFindEpisodes(
                    format!("{} {} {}", serie_id, season, number),
                    "update_episode".to_string(),
                )
            })?;
        self.connection
            .call(move |conn| {
                let mut where_query = QueryBuilder::new();

                where_query.add_update(&update.name, "name");
                where_query.add_update(&update.abs, "abs");
                where_query.add_update(&update.overview, "overview");
                where_query.add_update(&update.airdate, "airdate");
                where_query.add_update(&update.duration, "duration");
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

                let alts = replace_add_remove_from_array(
                    existing.alt,
                    update.alt,
                    update.add_alts,
                    update.remove_alts,
                );
                let alts = to_pipe_separated_optional(alts);
                where_query.add_update(&alts, "alt");

                where_query.add_where(QueryWhereType::Equal("serie_ref", &id));
                where_query.add_where(QueryWhereType::Equal("season", &season));
                where_query.add_where(QueryWhereType::Equal("number", &number));

                let update_sql = format!(
                    "UPDATE episodes SET {} {}",
                    where_query.format_update(),
                    where_query.format()
                );

                conn.execute(&update_sql, where_query.values())?;
                Ok(())
            })
            .await?;

        Ok(())
    }

    pub async fn add_episode(&self, episode: Episode) -> Result<()> {
        self.connection.call( move |conn| { 
            //println!("oo {} {} {}", episode.serie_ref, episode.season, episode.number);
            conn.execute("INSERT INTO episodes (serie_ref, season, number, abs, name, overview, airdate, duration, alt, params, imdb, slug, tmdb, trakt, tvdb, otherids, imdb_rating, imdb_votes, trakt_rating, trakt_votes)
            VALUES (?, ?, ? ,?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)", params![
                episode.serie,
                episode.season,
                episode.number,
                episode.abs,
                episode.name,
                episode.overview,
                episode.airdate,
                episode.duration,
                to_pipe_separated_optional(episode.alt),
                episode.params,
                episode.imdb,
                episode.slug,
                episode.tmdb,
                episode.trakt,
                episode.tvdb,
                episode.otherids,
                episode.imdb_rating,
                episode.imdb_votes,
                episode.trakt_rating,
                episode.trakt_votes
            ])?;
            Ok(())
        }).await?;
        Ok(())
    }

    pub async fn sync_serie_episodes(
        &self,
        serie_ref: &str,
        episodes: Vec<Episode>,
    ) -> Result<EpisodeSyncResult> {
        let serie_ref = serie_ref.to_string();
        let result = self
            .connection
            .call(move |conn| {
                let tx = conn.transaction()?;
                let existing: HashMap<(u32, u32), Episode> = {
                    let mut query = tx.prepare(
                        "SELECT serie_ref, season, number, abs, name, overview, airdate, duration, alt, params, imdb, slug, tmdb, trakt, tvdb, otherids, modified, added, imdb_rating, imdb_votes, trakt_rating, trakt_votes, null as serie_name
                         FROM episodes WHERE serie_ref = ? ORDER BY season, number",
                    )?;
                    let rows = query.query_map([&serie_ref], Self::row_to_episode)?;
                    rows.collect::<std::result::Result<Vec<_>, _>>()?
                        .into_iter()
                        .map(|episode| ((episode.season, episode.number), episode))
                        .collect()
                };
                let incoming_keys: HashSet<(u32, u32)> = episodes
                    .iter()
                    .map(|episode| (episode.season, episode.number))
                    .collect();
                let mut changed_keys: HashMap<(u32, u32), ElementAction> = HashMap::new();

                for episode in episodes {
                    let key = (episode.season, episode.number);
                    let existed = existing.contains_key(&key);
                    let changed = tx.execute(
                        "INSERT INTO episodes (
                            serie_ref, season, number, abs, name, overview, airdate, duration,
                            alt, params, imdb, slug, tmdb, trakt, tvdb, otherids,
                            imdb_rating, imdb_votes, trakt_rating, trakt_votes
                         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                         ON CONFLICT(serie_ref, season, number) DO UPDATE SET
                            abs = excluded.abs,
                            name = excluded.name,
                            overview = excluded.overview,
                            airdate = excluded.airdate,
                            duration = excluded.duration,
                            alt = excluded.alt,
                            params = excluded.params,
                            imdb = excluded.imdb,
                            slug = excluded.slug,
                            tmdb = excluded.tmdb,
                            trakt = excluded.trakt,
                            tvdb = excluded.tvdb,
                            otherids = excluded.otherids,
                            imdb_rating = excluded.imdb_rating,
                            imdb_votes = excluded.imdb_votes,
                            trakt_rating = excluded.trakt_rating,
                            trakt_votes = excluded.trakt_votes
                         WHERE episodes.abs IS NOT excluded.abs
                            OR episodes.name IS NOT excluded.name
                            OR episodes.overview IS NOT excluded.overview
                            OR episodes.airdate IS NOT excluded.airdate
                            OR episodes.duration IS NOT excluded.duration
                            OR episodes.alt IS NOT excluded.alt
                            OR episodes.params IS NOT excluded.params
                            OR episodes.imdb IS NOT excluded.imdb
                            OR episodes.slug IS NOT excluded.slug
                            OR episodes.tmdb IS NOT excluded.tmdb
                            OR episodes.trakt IS NOT excluded.trakt
                            OR episodes.tvdb IS NOT excluded.tvdb
                            OR episodes.otherids IS NOT excluded.otherids
                            OR episodes.imdb_rating IS NOT excluded.imdb_rating
                            OR episodes.imdb_votes IS NOT excluded.imdb_votes
                            OR episodes.trakt_rating IS NOT excluded.trakt_rating
                            OR episodes.trakt_votes IS NOT excluded.trakt_votes",
                        params![
                            episode.serie,
                            episode.season,
                            episode.number,
                            episode.abs,
                            episode.name,
                            episode.overview,
                            episode.airdate,
                            episode.duration,
                            to_pipe_separated_optional(episode.alt),
                            episode.params,
                            episode.imdb,
                            episode.slug,
                            episode.tmdb,
                            episode.trakt,
                            episode.tvdb,
                            episode.otherids,
                            episode.imdb_rating,
                            episode.imdb_votes,
                            episode.trakt_rating,
                            episode.trakt_votes,
                        ],
                    )?;
                    if changed > 0 {
                        changed_keys.insert(
                            key,
                            if existed {
                                ElementAction::Updated
                            } else {
                                ElementAction::Added
                            },
                        );
                    }
                }

                let mut deleted = Vec::new();
                for (key, episode) in &existing {
                    if incoming_keys.contains(key) {
                        continue;
                    }
                    let removed = tx.execute(
                        "DELETE FROM episodes
                         WHERE serie_ref = ? AND season = ? AND number = ?
                           AND NOT EXISTS (
                               SELECT 1 FROM media_serie_mapping
                               WHERE serie_ref = ? AND season = ? AND episode = ?
                           )",
                        params![
                            serie_ref,
                            key.0,
                            key.1,
                            serie_ref,
                            key.0,
                            key.1,
                        ],
                    )?;
                    if removed > 0 {
                        deleted.push(EpisodeWithAction {
                            action: ElementAction::Deleted,
                            episode: episode.clone(),
                        });
                    }
                }

                let refreshed = {
                    let mut query = tx.prepare(
                        "SELECT serie_ref, season, number, abs, name, overview, airdate, duration, alt, params, imdb, slug, tmdb, trakt, tvdb, otherids, modified, added, imdb_rating, imdb_votes, trakt_rating, trakt_votes, null as serie_name
                         FROM episodes WHERE serie_ref = ? ORDER BY season, number",
                    )?;
                    let rows = query.query_map([&serie_ref], Self::row_to_episode)?;
                    rows.collect::<std::result::Result<Vec<_>, _>>()?
                };
                let mut changes: Vec<EpisodeWithAction> = refreshed
                    .iter()
                    .filter_map(|episode| {
                        changed_keys
                            .remove(&(episode.season, episode.number))
                            .map(|action| EpisodeWithAction {
                                action,
                                episode: episode.clone(),
                            })
                    })
                    .collect();
                changes.extend(deleted);
                tx.commit()?;
                Ok(EpisodeSyncResult {
                    episodes: refreshed,
                    changes,
                })
            })
            .await?;
        Ok(result)
    }

    pub async fn remove_episode(&self, serie_ref: String, season: u32, number: u32) -> Result<()> {
        self.connection
            .call(move |conn| {
                conn.execute(
                    "DELETE FROM episodes WHERE serie_ref = ? and season = ? and number = ?",
                    params![serie_ref, season, number],
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
    use crate::{domain::episode::Episode, domain::ElementAction, model::episodes::EpisodeQuery};

    fn episode(number: u32, name: &str) -> Episode {
        Episode {
            serie: "serie-1".to_string(),
            season: 1,
            number,
            name: Some(name.to_string()),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn episode_sync_upserts_in_one_snapshot_and_preserves_media_mappings() {
        let connection = tokio_rusqlite::Connection::open_in_memory().await.unwrap();
        let store = SqliteLibraryStore::new(connection).await.unwrap();
        store.add_episode(episode(1, "Old title")).await.unwrap();
        store.add_episode(episode(2, "Mapped episode")).await.unwrap();
        store.add_episode(episode(4, "Removed episode")).await.unwrap();
        let original_modified = store
            .get_episode("serie-1", 1, 1)
            .await
            .unwrap()
            .unwrap()
            .modified;
        store
            .connection
            .call(|conn| {
                conn.execute(
                    "INSERT INTO media_serie_mapping (media_ref, serie_ref, season, episode)
                     VALUES ('media-1', 'serie-1', 1, 2)",
                    [],
                )?;
                Ok(())
            })
            .await
            .unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        let incoming = vec![episode(1, "New title"), episode(3, "New episode")];
        let sync = store
            .sync_serie_episodes("serie-1", incoming.clone())
            .await
            .unwrap();

        assert_eq!(
            sync.episodes
                .iter()
                .map(|episode| episode.number)
                .collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
        assert!(
            sync.episodes
                .iter()
                .find(|episode| episode.number == 1)
                .unwrap()
                .modified
                > original_modified,
            "metadata updates with a null abs must advance modified"
        );
        assert!(sync.changes.iter().any(|change| {
            change.episode.number == 1 && matches!(change.action, ElementAction::Updated)
        }));
        assert!(sync.changes.iter().any(|change| {
            change.episode.number == 3 && matches!(change.action, ElementAction::Added)
        }));
        assert!(sync.changes.iter().any(|change| {
            change.episode.number == 4 && matches!(change.action, ElementAction::Deleted)
        }));

        let mapping_count = store
            .connection
            .call(|conn| {
                Ok(conn.query_row(
                    "SELECT COUNT(*) FROM media_serie_mapping
                     WHERE media_ref = 'media-1' AND serie_ref = 'serie-1'
                       AND season = 1 AND episode = 2",
                    [],
                    |row| row.get::<_, u32>(0),
                )?)
            })
            .await
            .unwrap();
        assert_eq!(mapping_count, 1);

        let unchanged = store
            .sync_serie_episodes("serie-1", incoming)
            .await
            .unwrap();
        assert!(unchanged.changes.is_empty());
    }

    #[tokio::test]
    async fn episode_query_applies_stable_limit_and_offset() {
        let connection = tokio_rusqlite::Connection::open_in_memory().await.unwrap();
        let store = SqliteLibraryStore::new(connection).await.unwrap();
        store.add_episode(episode(1, "One")).await.unwrap();
        store.add_episode(episode(2, "Two")).await.unwrap();
        store.add_episode(episode(3, "Three")).await.unwrap();

        let page = store
            .get_episodes(EpisodeQuery {
                limit: Some(1),
                offset: Some(1),
                ..Default::default()
            })
            .await
            .unwrap();

        assert_eq!(page.len(), 1);
        assert_eq!(page[0].number, 2);
    }
}

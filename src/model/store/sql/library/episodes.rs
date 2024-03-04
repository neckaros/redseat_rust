use rusqlite::{params, OptionalExtension, Row};

use crate::{domain::episode::{self, Episode}, model::{episodes::{EpisodeForAdd, EpisodeForUpdate, EpisodeQuery}, store::{from_pipe_separated_optional, sql::{OrderBuilder, QueryBuilder, QueryWhereType, SqlOrder}, to_pipe_separated_optional}}, tools::array_tools::replace_add_remove_from_array};
use super::{Result, SqliteLibraryStore};
use crate::model::Error;



impl SqliteLibraryStore {
  
    fn row_to_episode(row: &Row) -> rusqlite::Result<Episode> {
        Ok(Episode {
            serie_ref: row.get(0)?,
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
        })
    }

    pub async fn get_episodes(&self, query: EpisodeQuery) -> Result<Vec<Episode>> {
        let row = self.connection.call( move |conn| { 
            let mut where_query = QueryBuilder::new();
            where_query.add_where(query.after, QueryWhereType::After("modified".to_string()));

            where_query.add_where(query.serie_ref, QueryWhereType::Equal("serie_ref".to_string()));
            where_query.add_where(query.season, QueryWhereType::Equal("season".to_string()));

            if query.after.is_some() {
                where_query.add_oder(OrderBuilder::new("modified".to_string(), SqlOrder::ASC));
            } else {
                where_query.add_oder(OrderBuilder::new("season".to_string(), SqlOrder::ASC));
                where_query.add_oder(OrderBuilder::new("number".to_string(), SqlOrder::ASC));
            }


            let mut query = conn.prepare(&format!("SELECT serie_ref, season, number, abs, name, overview, airdate, duration, alt, params, imdb, slug, tmdb, trakt, tvdb, otherids, modified, added, imdb_rating, imdb_votes, trakt_rating, trakt_votes  FROM episodes {}{}", where_query.format(), where_query.format_order()))?;
            let rows = query.query_map(
            where_query.values(), Self::row_to_episode,
            )?;
            let backups:Vec<Episode> = rows.collect::<std::result::Result<Vec<Episode>, rusqlite::Error>>()?; 
            Ok(backups)
        }).await?;
        Ok(row)
    }
    pub async fn get_episode(&self, serie_id: &str, season: usize, number: usize) -> Result<Option<Episode>> {
        let serie_id = serie_id.to_string();
        let row = self.connection.call( move |conn| { 
            let mut query = conn.prepare("SELECT serie_ref, season, number, abs, name, overview, airdate, duration, alt, params, imdb, slug, tmdb, trakt, tvdb, otherids, modified, added, imdb_rating, imdb_votes, trakt_rating, trakt_votes FROM episodes WHERE serie_ref = ? and season = ? and number = ?")?;
            let row = query.query_row(
            params![serie_id, season, number],Self::row_to_episode).optional()?;
            Ok(row)
        }).await?;
        Ok(row)
    }



    pub async fn update_episode(&self, serie_id: &str, season: usize, number: usize, update: EpisodeForUpdate) -> Result<()> {
        let id = serie_id.to_string();
        let existing = self.get_episode(serie_id, season, number).await?.ok_or_else( || Error::NotFound)?;
        self.connection.call( move |conn| { 
            let mut where_query = QueryBuilder::new();
            
            where_query.add_update(update.abs.clone(), QueryWhereType::Equal("abs".to_string()));
            where_query.add_update(update.name.clone(), QueryWhereType::Equal("name".to_string()));
            where_query.add_update(update.overview.clone(), QueryWhereType::Equal("overview".to_string()));
            where_query.add_update(update.airdate.clone(), QueryWhereType::Equal("airdate".to_string()));
            where_query.add_update(update.duration.clone(), QueryWhereType::Equal("duration".to_string()));
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


            let alts = replace_add_remove_from_array(existing.alt, update.alt, update.add_alts, update.remove_alts);
            where_query.add_update(to_pipe_separated_optional(alts), QueryWhereType::Equal("alt".to_string()));

            where_query.add_where(Some(id), QueryWhereType::Equal("serie_ref".to_string()));
            where_query.add_where(Some(season), QueryWhereType::Equal("season".to_string()));
            where_query.add_where(Some(number), QueryWhereType::Equal("number".to_string()));
            

            let update_sql = format!("UPDATE episodes SET {} {}", where_query.format_update(), where_query.format());

            conn.execute(&update_sql, where_query.values())?;
            Ok(())
        }).await?;

        Ok(())
    }

    pub async fn add_episode(&self, episode: EpisodeForAdd) -> Result<()> {
        self.connection.call( move |conn| { 

            conn.execute("INSERT INTO episodes (serie_ref, season, number, abs, name, overview, airdate, duration, alt, params, imdb, slug, tmdb, trakt, tvdb, otherids, imdb_rating, imdb_votes, trakt_rating, trakt_votes)
            VALUES (?, ?, ? ,?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)", params![
                episode.serie_ref,
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

    pub async fn remove_episode(&self, episode_id: String, season: usize, number: usize) -> Result<()> {
        self.connection.call( move |conn| { 
            conn.execute("DELETE FROM episodes WHERE serie_ref = ? and season = ? and number = ?", params![episode_id, season, number])?;
            Ok(())
        }).await?;
        Ok(())
    }
}
use axum::{async_trait, Error};
use chrono::DateTime;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use crate::domain::movie::Movie;
use crate::domain::serie::{Serie, SerieStatus};
use crate::model::episodes::{EpisodeForUpdate, EpisodeQuery};
use crate::model::movies::MovieQuery;
use crate::model::series::SerieForUpdate;
use crate::tools::clock::{now, Clock};
use crate::{domain::library, error::RsResult, model::{series::SerieQuery, users::ConnectedUser, ModelController}, plugins::sources::Source, tools::{clock::UtcDate, log::{log_error, log_info}}};

use super::RsSchedulerTask;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct BackupTask {
    pub specific_library: Option<String>
}

impl BackupTask {

}

#[async_trait]
impl RsSchedulerTask for BackupTask {
    async fn execute(&self, mc: ModelController) -> RsResult<()> {
        let connected_user = &ConnectedUser::ServerAdmin;
        let libraries = mc.get_libraries(&connected_user).await?;
        let libraries = if let Some(specific_library) = &self.specific_library {
            libraries.into_iter().filter(|l| &l.id == specific_library).collect()
        } else {
            libraries
        };

        for library in libraries {
            log_info(crate::tools::log::LogServiceType::Scheduler, format!("Refreshing library {:?}", library.name));
            match library.kind {
                library::LibraryType::Photos => {},
                library::LibraryType::Shows => {
                    let refresh_path = "settings/trakt_serie_refresh.txt";
                    let source = mc.library_source_for_library(&library.id).await?;
                    let last_update = if let Ok(mut data) = source.get_file_library(refresh_path,).await {
                        let mut buffer = String::new();
                        data.read_to_string(&mut buffer).await?;
                        DateTime::parse_from_rfc3339(&buffer).ok()
                    } else {
                        None
                    };
                    let now = now().floor_to_hour().ok_or(crate::error::Error::TimeCreationError)?;
                    
                    let series: Vec<Serie> = mc.get_series(&library.id, SerieQuery::new_empty(), &connected_user).await?.into_iter().filter(|s| s.trakt.is_some()).collect();
                    let series: Vec<Serie> = if let Some(last_update) = last_update {
                        let to_refresh = mc.trakt.episodes_refreshed(last_update).await;
                        if let Ok(to_refresh) = to_refresh {
                            series.into_iter().filter(|s| to_refresh.contains(&s.trakt.unwrap_or(0))).collect()
                        } else {
                            log_info(crate::tools::log::LogServiceType::Scheduler, "Too many page will refresh all".to_owned());
                            series
                        }
                    } else {
                        series
                    };

                    for serie in series {
                        let refreshed = mc.refresh_serie(&library.id, &serie.id, &connected_user).await;
                        if let Err(refreshed) = refreshed {
                            log_error(crate::tools::log::LogServiceType::Scheduler, format!("Error refreshing serie {}: {:#}", serie.name, refreshed));
                        } else {
                            log_info(crate::tools::log::LogServiceType::Scheduler, format!("Refreshed serie {}", serie.name));
                        }
                        if serie.status != Some(SerieStatus::Ended) && serie.status != Some(SerieStatus::Canceled) {
                            let episodes = mc.refresh_episodes(&library.id, &serie.id, &connected_user).await;
                            if let Err(error) = episodes {
                                log_error(crate::tools::log::LogServiceType::Scheduler, format!("Error refreshing serie {}: {:#}", serie.name, error));
                            } else if let Ok(episodes) = episodes {
                                log_info(crate::tools::log::LogServiceType::Scheduler, format!("Refreshed serie {} espisodes ({})", serie.name, episodes.len()));
                            }
                        }
                    }
                    let mut stream = source.get_file_write_library_overwrite(&refresh_path).await?;

                    //Imdb rating
                    mc.refresh_series_imdb(&library.id, &ConnectedUser::ServerAdmin).await?;

                    stream.write_all(now.print().as_bytes()).await?;
                    stream.flush().await?
                    
                },
                library::LibraryType::Movies => {
                    let refresh_path = "settings/trakt_movie_refresh.txt";
                    let source = mc.library_source_for_library(&library.id).await?;
                    let last_update = if let Ok(mut data) = source.get_file_library(refresh_path,).await {
                        let mut buffer = String::new();
                        data.read_to_string(&mut buffer).await?;
                        DateTime::parse_from_rfc3339(&buffer).ok()
                    } else {
                        None
                    };
                    let now = now().floor_to_hour().ok_or(crate::error::Error::TimeCreationError)?;
                    
                    let movies: Vec<Movie> = mc.get_movies(&library.id, MovieQuery::new_empty(), &connected_user).await?.into_iter().filter(|s| s.trakt.is_some()).collect();
                    let movies: Vec<Movie> = if let Some(last_update) = last_update {
                        let to_refresh = mc.trakt.movies_refreshed(last_update).await;
                        if let Ok(to_refresh) = to_refresh {
                            movies.into_iter().filter(|s| to_refresh.contains(&s.trakt.unwrap_or(0))).collect()
                        } else {
                            log_info(crate::tools::log::LogServiceType::Scheduler, "Too many page will refresh all".to_owned());
                            movies
                        }
                    } else {
                        movies
                    };

                    for movie in movies {
                        let refreshed = mc.refresh_movie(&library.id, &movie.id, &connected_user).await;
                        if let Err(refreshed) = refreshed {
                            log_error(crate::tools::log::LogServiceType::Scheduler, format!("Error refreshing movie {}: {:#}", movie.name, refreshed));
                        } else {
                            log_info(crate::tools::log::LogServiceType::Scheduler, format!("Refreshed movie {}", movie.name));
                        }
                    }

                    //Imdb rating
                    mc.refresh_movies_imdb(&library.id, &ConnectedUser::ServerAdmin).await?;

                    let mut stream = source.get_file_write_library_overwrite(&refresh_path).await?;

                    stream.write_all(now.print().as_bytes()).await?;
                    stream.flush().await?
                },
                library::LibraryType::Iptv => {},
                library::LibraryType::Other => {},
            }
        }
                
        log_info(crate::tools::log::LogServiceType::Scheduler, format!("Refreshed all"));
        Ok(())
    }
}
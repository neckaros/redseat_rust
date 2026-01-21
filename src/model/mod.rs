pub mod backups;
pub mod credentials;
pub mod error;
pub mod libraries;
pub mod plugins;
pub mod server;
pub mod store;
pub mod users;

pub mod deleted;
pub mod episodes;
pub mod media_progresses;
pub mod media_ratings;
pub mod medias;
pub mod movies;
pub mod people;
pub mod series;
pub mod tags;

use crate::{
    domain::{
        backup::BackupProcessStatus,
        library::{LibraryMessage, LibraryRole, LibraryStatusMessage, ServerLibrary},
        media::ConvertProgress,
    },
    error::{RsError, RsResult},
    plugins::{
        medias::{
            fanart::FanArtContext, imdb::ImdbContext, tmdb::TmdbContext, trakt::TraktContext,
        },
        sources::{
            error::SourcesError, local_provider_for_library, path_provider::PathProvider,
            AsyncReadPinBox, FileStreamResult, Source, SourceRead,
        },
        PluginManager,
    },
    tools::{
        clock::SECONDS_IN_HOUR,
        image_tools::{resize_image_reader, ImageSize},
        log::log_info,
        scheduler::{
            self, face_recognition::FaceRecognitionTask, ip::RefreshIpTask, refresh::RefreshTask,
            request_progress::RequestProgressTask, RsScheduler, RsTaskType,
        },
    },
};
use futures::lock::Mutex;
use nanoid::nanoid;
use rs_plugin_common_interfaces::{
    video::{RsVideoTranscodeStatus, VideoConvertRequest},
    ImageType, RsRequest,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, VecDeque},
    io::Read,
    path::PathBuf,
    pin::Pin,
    sync::{Arc, RwLock as StdRwLock},
    thread::JoinHandle,
};
use strum::IntoEnumIterator;

use self::{
    medias::CRYPTO_HEADER_SIZE,
    store::SqliteStore,
    users::{ConnectedUser, ServerUser, UserRole},
};
use error::{Error, Result};
use tokio::{
    fs::{self, remove_file, File},
    io::{copy, AsyncRead, BufReader},
    sync::{broadcast, RwLock},
};

use crate::routes::sse::SseEvent;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct VideoConvertQueueElement {
    request: VideoConvertRequest,
    library: String,
    media: String,
    user: ConnectedUser,
    id: String,
    plugin_id: Option<String>,
    status: ConvertProgress,
}

impl VideoConvertQueueElement {
    pub fn new(
        library: String,
        plugin_id: Option<String>,
        media: String,
        filename: String,
        user: ConnectedUser,
        request: VideoConvertRequest,
    ) -> VideoConvertQueueElement {
        VideoConvertQueueElement {
            id: request.id.clone(),
            plugin_id,
            status: ConvertProgress {
                id: request.id.clone(),
                filename,
                converted_id: None,
                done: false,
                percent: 0f64,
                status: RsVideoTranscodeStatus::Pending,
                estimated_remaining_seconds: None,
                request: Some(request.clone()),
            },
            request,
            library,
            media,
            user,
        }
    }
}

#[derive(Clone)]
pub struct ModelController {
    store: Arc<SqliteStore>,
    pub plugin_manager: Arc<PluginManager>,
    pub trakt: Arc<TraktContext>,
    pub tmdb: Arc<TmdbContext>,
    pub fanart: Arc<FanArtContext>,
    pub imdb: Arc<ImdbContext>,
    pub scheduler: Arc<RsScheduler>,

    pub convert_queue: Arc<RwLock<VecDeque<VideoConvertQueueElement>>>,
    pub convert_current: Arc<RwLock<bool>>,
    pub convert_current_process: Arc<RwLock<Option<JoinHandle<()>>>>,

    pub backup_processes: Arc<RwLock<Vec<BackupProcessStatus>>>,

    pub chache_libraries: Arc<RwLock<HashMap<String, ServerLibrary>>>,

    /// Broadcast channel for SSE events
    pub sse_tx: broadcast::Sender<SseEvent>,
}

// Constructor
impl ModelController {
    pub async fn new(store: SqliteStore, plugin_manager: PluginManager) -> crate::Result<Self> {
        let tmdb = TmdbContext::new("4a01db3a73eed5cf17e9c7c27fd9d008".to_string()).await?;
        let fanart = FanArtContext::new("a6eb2f1acb7b54550e498a9b37a574fa".to_string());
        let scheduler = RsScheduler::new();
        let (sse_tx, _) = broadcast::channel::<SseEvent>(1024);

        let mc = Self {
            store: Arc::new(store),
            plugin_manager: Arc::new(plugin_manager),
            trakt: Arc::new(TraktContext::new(
                "455f81b3409a8dd140a941e9250ff22b2ed92d68003491c3976363fe752a9024".to_string(),
            )),
            tmdb: Arc::new(tmdb),
            fanart: Arc::new(fanart),
            imdb: Arc::new(ImdbContext::new()),
            scheduler: Arc::new(scheduler),
            chache_libraries: Arc::new(RwLock::new(HashMap::new())),
            convert_queue: Arc::new(RwLock::new(VecDeque::new())),
            convert_current: Arc::new(RwLock::new(false)),
            convert_current_process: Arc::new(RwLock::new(None)),

            backup_processes: Arc::new(RwLock::new(vec![])),
            sse_tx,
        };

        let pm_forload = mc.plugin_manager.clone();
        tokio::spawn(async move {
            pm_forload.reload().await.unwrap();
        });

        mc.cache_update_all_libraries().await?;

        let scheduler = &mc.scheduler;
        scheduler.start(mc.clone()).await?;

        scheduler
            .add(
                RsTaskType::Refresh,
                scheduler::RsSchedulerWhen::Every(SECONDS_IN_HOUR),
                RefreshTask {
                    specific_library: None,
                },
            )
            .await?;
        scheduler
            .add(
                RsTaskType::Ip,
                scheduler::RsSchedulerWhen::Every(SECONDS_IN_HOUR / 2),
                RefreshIpTask {},
            )
            .await?;
        scheduler
            .add(
                RsTaskType::RequestProgress,
                scheduler::RsSchedulerWhen::Every(30),
                RequestProgressTask::new(),
            )
            .await?;
        //scheduler.add(RsTaskType::Face, scheduler::RsSchedulerWhen::Every(SECONDS_IN_HOUR * 3), FaceRecognitionTask {specific_library:None} ).await?;
        //scheduler.add(RsTaskType::Refresh, scheduler::RsSchedulerWhen::At(0), RefreshTask {specific_library:None} ).await?;
        //scheduler.tick(mc.clone()).await;
        Ok(mc)
    }
}

impl ModelController {
    pub async fn cache_get_library(&self, library: &str) -> Option<ServerLibrary> {
        let cache = self.chache_libraries.read().await;
        cache.get(library).cloned()
    }
    pub async fn cache_get_library_crypt(&self, library: &str) -> bool {
        let cache = self.chache_libraries.read().await;
        cache.get(library).and_then(|r| r.crypt).unwrap_or(false)
    }
    pub async fn cache_check_library_notcrypt(&self, library: &str) -> RsResult<()> {
        if self.cache_get_library_crypt(library).await {
            Err(crate::Error::UnavailableForCryptedLibraries)
        } else {
            Ok(())
        }
    }

    pub async fn cache_update_library(&self, library: ServerLibrary) {
        let mut cache = self.chache_libraries.write().await;
        cache.remove(&library.id);
        cache.insert(library.id.clone(), library);
    }
    pub async fn cache_remove_library(&self, library: &str) {
        let mut cache = self.chache_libraries.write().await;
        cache.remove(library);
    }
    pub async fn cache_update_all_libraries(&self) -> RsResult<()> {
        let libraries = self.store.get_libraries().await?;
        for library in libraries {
            self.cache_update_library(library).await;
        }
        Ok(())
    }

    pub async fn get_user_unchecked(&self, user_id: &str) -> Result<users::ServerUser> {
        self.store.get_user(user_id).await
    }

    pub async fn get_user(
        &self,
        user_id: &str,
        requesting_user: &ConnectedUser,
    ) -> Result<users::ServerUser> {
        let id = user_id.to_string();
        if let ConnectedUser::Anonymous = requesting_user {
            return Err(Error::UserGetNotAuth {
                user: requesting_user.clone(),
                requested_user: id,
            });
        } else if let ConnectedUser::Server(user) = &requesting_user {
            if user.id != id && user.role != UserRole::Admin {
                return Err(Error::UserGetNotAuth {
                    user: requesting_user.clone(),
                    requested_user: id,
                });
            }
        }

        self.store.get_user(&id).await
    }

    pub async fn add_user(
        &self,
        user: ServerUser,
        requesting_user: &ConnectedUser,
    ) -> Result<ServerUser> {
        requesting_user.check_role(&UserRole::Admin)?;
        let user_id = user.id.clone();
        self.store.add_user(user).await?;
        self.get_user(&user_id, requesting_user).await
    }

    pub async fn get_users(
        &self,
        requesting_user: &ConnectedUser,
    ) -> Result<Vec<users::ServerUser>> {
        if requesting_user.is_admin() {
            self.store.get_users().await
        } else {
            Err(Error::UserListNotAuth {
                user: requesting_user.clone(),
            })
        }
    }

    pub async fn source_for_library(&self, library_id: &str) -> RsResult<Box<dyn Source>> {
        let library = self.store.get_library(library_id).await?.ok_or_else(|| {
            Error::LibraryNotFoundFor(library_id.to_string(), "source_for_library".to_string())
        })?;
        let source = self
            .plugin_manager
            .source_for_library(library, self.clone())
            .await?;
        Ok(source)
    }
    pub async fn library_source_for_library(&self, library_id: &str) -> Result<PathProvider> {
        let library = self.store.get_library(library_id).await?.ok_or_else(|| {
            Error::LibraryNotFoundFor(
                library_id.to_string(),
                "library_source_for_library".to_string(),
            )
        })?;

        local_provider_for_library(&library)
            .await
            .map_err(|_| crate::model::Error::Other("Unable to get local provider".to_string()))
    }

    pub async fn library_image(
        &self,
        library_id: &str,
        folder: &str,
        id: &str,
        kind: Option<ImageType>,
        size: Option<ImageSize>,
        requesting_user: &ConnectedUser,
    ) -> RsResult<FileStreamResult<AsyncReadPinBox>> {
        requesting_user.check_library_role(library_id, LibraryRole::Read)?;
        self.cache_check_library_notcrypt(library_id).await?;

        let m = self.library_source_for_library(&library_id).await?;
        let mut source_filepath = format!(
            "{}/{}{}{}.avif",
            folder,
            id,
            ImageType::optional_to_filename_element(&kind),
            ImageSize::optional_to_filename_element(&size)
        );
        let reader_response = m.get_file(&source_filepath, None).await;

        if let Some(int_size) = size.clone() {
            if let Err(error) = &reader_response {
                if matches!(error, RsError::Source(SourcesError::NotFound(_))) {
                    let mut original_filepath = format!(
                        "{}/{}{}.avif",
                        folder,
                        id,
                        ImageType::optional_to_filename_element(&kind)
                    );
                    let exist = m.exists(&original_filepath).await;
                    if exist {
                        log_info(
                            crate::tools::log::LogServiceType::Other,
                            format!(
                                "Creating image size: {} {} {} {}. Original: {:?} to:{:?}",
                                folder,
                                id,
                                ImageType::optional_to_filename_element(&kind),
                                int_size,
                                &m.get_full_path(&original_filepath),
                                m.get_full_path(&source_filepath)
                            ),
                        );

                        let reader = m.get_file(&original_filepath, None).await?;
                        let mut reader = reader
                            .into_reader(
                                Some(library_id),
                                None,
                                None,
                                Some((self.clone(), &requesting_user)),
                                None,
                            )
                            .await?;
                        let image = resize_image_reader(
                            reader.stream,
                            512,
                            image::ImageFormat::Avif,
                            Some(50),
                            false,
                        )
                        .await?;
                        self.update_library_image(
                            &library_id,
                            folder,
                            id,
                            &kind,
                            &size,
                            image.as_slice(),
                            requesting_user,
                        )
                        .await?;

                        log_info(
                            crate::tools::log::LogServiceType::Other,
                            format!(
                                "image size created: {} {} {} {}",
                                folder,
                                id,
                                ImageType::optional_to_filename_element(&kind),
                                int_size
                            ),
                        );
                        let reader = m.get_file(&source_filepath, None).await?;

                        if let SourceRead::Stream(reader) = reader {
                            return Ok(reader);
                        } else {
                            return Err(SourcesError::NotFound(Some(format!(
                                "library_image - File Not Found - {}",
                                source_filepath
                            )))
                            .into());
                        }
                    }
                }
            }
        }
        let reader = reader_response?;
        if reader.size().unwrap_or(200) == 0 {
            m.remove(&source_filepath).await?;
            return Err(RsError::CorruptedImage);
        }
        if let SourceRead::Stream(reader) = reader {
            return Ok(reader);
        } else {
            return Err(crate::Error::ImageNotFound(
                format!("id:{} kind:{:?}", id, kind),
                "library_image".to_string(),
            ));
        }
    }
    pub async fn has_library_image(
        &self,
        library_id: &str,
        folder: &str,
        id: &str,
        kind: Option<ImageType>,
        requesting_user: &ConnectedUser,
    ) -> Result<bool> {
        requesting_user.check_library_role(library_id, LibraryRole::Read)?;

        let m = self.library_source_for_library(&library_id).await?;
        let source_filepath = format!(
            "{}/{}{}.avif",
            folder,
            id,
            ImageType::optional_to_filename_element(&kind)
        );
        let exist = m.exists(&source_filepath).await;
        Ok(exist)
    }
    pub async fn update_library_image<T: AsyncRead>(
        &self,
        library_id: &str,
        folder: &str,
        id: &str,
        kind: &Option<ImageType>,
        size: &Option<ImageSize>,
        reader: T,
        requesting_user: &ConnectedUser,
    ) -> Result<()> {
        requesting_user.check_library_role(library_id, LibraryRole::Write)?;
        self.remove_library_image(library_id, folder, id, kind, size, requesting_user)
            .await?;

        let m = self.library_source_for_library(&library_id).await?;

        let source_filepath = format!(
            "{}/{}{}{}.avif",
            folder,
            id,
            ImageType::optional_to_filename_element(&kind),
            ImageSize::optional_to_filename_element(&size)
        );

        let (_, writer) = m.get_file_write_stream(&source_filepath).await?;
        tokio::pin!(reader);
        tokio::pin!(writer);
        copy(&mut reader, &mut writer).await?;

        Ok(())
    }

    pub async fn remove_library_image(
        &self,
        library_id: &str,
        folder: &str,
        id: &str,
        kind: &Option<ImageType>,
        size: &Option<ImageSize>,
        requesting_user: &ConnectedUser,
    ) -> Result<()> {
        requesting_user.check_library_role(library_id, LibraryRole::Write)?;

        let m = self.library_source_for_library(&library_id).await?;

        let source_filepath = format!(
            "{}/{}{}{}.avif",
            folder,
            id,
            ImageType::optional_to_filename_element(&kind),
            ImageSize::optional_to_filename_element(&size)
        );
        let r = m.remove(&source_filepath).await;
        if r.is_ok() {
            log_info(
                crate::tools::log::LogServiceType::Other,
                format!("Deleted image {}", source_filepath),
            );
        }

        if size.is_none() {
            //remove all sizes
            for size in ImageSize::iter() {
                let source_filepath = format!(
                    "{}/{}{}{}.avif",
                    folder,
                    id,
                    ImageType::optional_to_filename_element(&kind),
                    size.to_filename_element()
                );
                let r = m.remove(&source_filepath).await;
                if r.is_ok() {
                    log_info(
                        crate::tools::log::LogServiceType::Other,
                        format!("Deleted image {}", source_filepath),
                    );
                }
            }
        }
        Ok(())
    }
}

impl ModelController {
    /// Broadcasts an event to all SSE subscribers
    pub fn broadcast_sse(&self, event: SseEvent) {
        let _ = self.sse_tx.send(event);
    }

    pub fn send_library(&self, message: LibraryMessage) {
        self.broadcast_sse(SseEvent::Library(message));
    }

    pub fn send_library_status(&self, message: LibraryStatusMessage) {
        self.broadcast_sse(SseEvent::LibraryStatus(message));
    }
}

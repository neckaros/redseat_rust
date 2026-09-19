use std::{
    future::Future,
    io,
    path::PathBuf,
    pin::Pin,
    str::FromStr,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};

use axum::async_trait;
use bytes::Bytes;
use chrono::{Datelike, Utc};
use futures::{ready, AsyncReadExt, Stream, TryFutureExt, TryStreamExt};
use nanoid::nanoid;
use query_external_ip::SourceError;
use rs_plugin_common_interfaces::{
    provider::{RsProviderAddRequest, RsProviderPath},
    request::RsRequest,
};
use tokio::{
    fs::{create_dir_all, File},
    io::{AsyncRead, AsyncWrite, AsyncWriteExt, BufReader, BufWriter, ReadBuf},
    sync::watch,
};
use tokio_stream::StreamExt;
use tokio_util::io::ReaderStream;

use crate::{
    domain::{
        backup::Backup, library::ServerLibrary, media::MediaForUpdate, plugin::PluginWithCredential,
    },
    error::{RsError, RsResult},
    model::{users::ConnectedUser, ModelController},
    plugins::{
        sources::{
            path_provider::PathProvider, streaming_http_client, RsRequestHeader,
            TRANSFER_IDLE_TIMEOUT,
        },
        PluginManager,
    },
    routes::mw_range::RangeDefinition,
    server::get_server_file_path_array,
    Error,
};

use super::{
    error::{SourcesError, SourcesResult},
    local_provider, AsyncReadPinBox, AsyncSeekableWrite, BoxedStringFuture, FileStreamResult,
    Source, SourceRead,
};

fn stream_with_idle_timeout<R>(
    mut stream: ReaderStream<R>,
    progress: Option<watch::Sender<()>>,
) -> impl Stream<Item = io::Result<Bytes>>
where
    R: AsyncRead + Unpin + Send + 'static,
{
    async_stream::stream! {
        loop {
            match tokio::time::timeout(TRANSFER_IDLE_TIMEOUT, stream.next()).await {
                Ok(Some(chunk)) => {
                    if chunk.is_ok() {
                        if let Some(progress) = &progress {
                            progress.send_replace(());
                        }
                    }
                    yield chunk;
                }
                Ok(None) => break,
                Err(_) => {
                    yield Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "provider transfer made no progress before the idle timeout",
                    ));
                    break;
                }
            }
        }
    }
}

async fn wait_for_upload_with_idle_timeout<F, T>(
    upload: F,
    mut progress: watch::Receiver<()>,
    idle_timeout: Duration,
) -> RsResult<T>
where
    F: Future<Output = RsResult<T>>,
{
    tokio::pin!(upload);

    loop {
        tokio::select! {
            result = &mut upload => return result,
            progress_result = tokio::time::timeout(idle_timeout, progress.changed()) => {
                match progress_result {
                    Ok(Ok(())) => {}
                    // The request body finished. Reqwest's read timeout now protects the
                    // response wait, so the upload-progress watchdog is no longer needed.
                    Ok(Err(_)) => return upload.await,
                    Err(_) => {
                        return Err(io::Error::new(
                            io::ErrorKind::TimedOut,
                            "provider upload made no progress before the idle timeout",
                        ).into());
                    }
                }
            }
        }
    }
}

async fn send_upload_with_idle_timeout(
    request: reqwest::RequestBuilder,
    progress: watch::Receiver<()>,
) -> RsResult<reqwest::Response> {
    wait_for_upload_with_idle_timeout(
        async move { Ok(request.send().await?) },
        progress,
        TRANSFER_IDLE_TIMEOUT,
    )
    .await
}

fn map_provider_error(error: RsError, fallback: SourcesError) -> RsError {
    match error {
        timeout @ Error::PluginTimeout(_, _) => timeout,
        _ => fallback.into(),
    }
}

struct StagedUploadCleanup(PathBuf);

impl Drop for StagedUploadCleanup {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

pub struct PluginProvider {
    id: String,
    plugin: PluginWithCredential,
    plugin_manager: Arc<PluginManager>,
    root: String,
    data_path: Option<String>,
}

#[async_trait]
impl Source for PluginProvider {
    async fn new(library: ServerLibrary, controller: ModelController) -> RsResult<Self> {
        let plugin_id = library.plugin.clone().ok_or(SourcesError::Other(format!(
            "Plugin library need a plugin: {:?}",
            library
        )))?;
        let credential_id = library.credentials.clone();
        let plugin = controller
            .get_plugin(plugin_id, &ConnectedUser::ServerAdmin)
            .await
            .map_err(|_| {
                SourcesError::Other(format!("Plugin library need a plugin: {:?}", library))
            })?;
        let credential = if let Some(credential_id) = credential_id {
            controller
                .get_credential(credential_id, &ConnectedUser::ServerAdmin)
                .await
                .map_err(|_| {
                    SourcesError::Other(format!("Unable to get credential: {:?}", library))
                })?
        } else {
            None
        };
        let plugin_with_credentials = PluginWithCredential { plugin, credential };

        Ok(Self {
            id: library.id.clone(),
            root: library.root.clone().unwrap_or("/".to_string()),
            data_path: library.settings.data_path.clone(),
            plugin: plugin_with_credentials,
            plugin_manager: controller.plugin_manager.clone(),
        })
    }

    async fn new_from_backup(backup: Backup, controller: ModelController) -> RsResult<Self> {
        let plugin_id = backup.plugin.clone().ok_or(SourcesError::Other(format!(
            "Plugin backup need a plugin: {:?}",
            backup
        )))?;
        let credential_id = backup.credentials.clone();
        let plugin = controller
            .get_plugin(plugin_id, &ConnectedUser::ServerAdmin)
            .await
            .map_err(|_| {
                SourcesError::Other(format!("Plugin backup need a plugin: {:?}", backup))
            })?;
        let credential = if let Some(credential_id) = credential_id {
            controller
                .get_credential(credential_id, &ConnectedUser::ServerAdmin)
                .await
                .map_err(|_| {
                    SourcesError::Other(format!("Unable to get credential: {:?}", backup))
                })?
        } else {
            None
        };
        let plugin_with_credentials = PluginWithCredential { plugin, credential };

        Ok(PluginProvider {
            id: backup.id.clone(),
            root: backup.path,
            data_path: None,
            plugin: plugin_with_credentials,
            plugin_manager: controller.plugin_manager.clone(),
        })
    }

    async fn init(&self) -> SourcesResult<()> {
        let local = local_provider(
            &self.id,
            "PluginProvider",
            &Some(self.root.clone()),
            &self.data_path,
        )
        .await
        .map_err(|_| SourcesError::Other("Unable to init library".to_string()))?;

        local.init().await?;
        Ok(())
    }

    async fn exists(&self, _source: &str) -> bool {
        true
    }
    async fn remove(&self, source: &str) -> RsResult<()> {
        self.plugin_manager
            .provider_remove_file(
                RsProviderPath {
                    root: Some(self.root.clone()),
                    source: source.to_string(),
                },
                &self.plugin,
            )
            .await
    }

    fn local_path(&self, _source: &str) -> Option<PathBuf> {
        None
    }

    async fn fill_infos(&self, source: &str, infos: &mut MediaForUpdate) -> RsResult<()> {
        let entry = self
            .plugin_manager
            .provider_info_file(
                RsProviderPath {
                    root: Some(self.root.clone()),
                    source: source.to_string(),
                },
                &self.plugin,
            )
            .await?;
        if let Some(size) = entry.size {
            infos.size = Some(size);
        }
        if let Some(hash) = entry.hash {
            infos.md5 = Some(hash);
        }

        if let Some(mime) = entry.mimetype {
            infos.mimetype = Some(mime);
        }
        if let Some(created) = entry.created {
            infos.created = Some(created);
        }
        if let Some(modified) = entry.modified {
            infos.modified = Some(modified);
        }
        Ok(())
    }
    async fn get_file(
        &self,
        source: &str,
        _range: Option<RangeDefinition>,
    ) -> RsResult<SourceRead> {
        //println!("root: {}, source: {}", self.root, source);
        let request = self
            .plugin_manager
            .provider_get_file(
                RsProviderPath {
                    root: Some(self.root.clone()),
                    source: source.to_string(),
                },
                &self.plugin,
            )
            .await?;
        Ok(SourceRead::Request(request))
    }

    async fn writerseek(
        &self,
        name: &str,
    ) -> RsResult<(String, Pin<Box<dyn AsyncSeekableWrite + Send>>)> {
        Err(crate::Error::NotImplemented(
            "Writerseek not implemented for plugin provider".to_string(),
        ))
    }

    async fn writer(
        &self,
        name: &str,
        length: Option<u64>,
        mime: Option<String>,
    ) -> RsResult<(BoxedStringFuture, Pin<Box<dyn AsyncWrite + Send>>)> {
        let (asyncwriter, asyncreader) = tokio::io::duplex(256 * 1024);
        let mut streamreader = tokio_util::io::ReaderStream::new(asyncreader);

        let request = self
            .plugin_manager
            .provider_upload_file_request(
                RsProviderAddRequest {
                    root: self.root.clone(),
                    name: name.to_string(),
                    overwrite: false,
                },
                &self.plugin,
            )
            .await
            .map_err(|error| {
                map_provider_error(error, SourcesError::NotFound(Some(name.to_string())))
            })?;

        let content_length = length.clone();
        let mime = mime
            .unwrap_or("application/octet-stream".to_string())
            .to_string();
        let plugin = self.plugin.clone();
        let local = local_provider(
            &self.id,
            "PluginProvider",
            &Some(self.root.clone()),
            &self.data_path,
        )
        .await?;
        let filename = name.to_string();
        let plugin_manager = self.plugin_manager.clone();
        let source = tokio::spawn(async move {
            if let Some(length) = content_length {
                let (progress_tx, progress_rx) = watch::channel(());
                let body = reqwest::Body::wrap_stream(stream_with_idle_timeout(
                    streamreader,
                    Some(progress_tx),
                ));
                let client = streaming_http_client()?;
                //println!("sending to stream (size: {}) {}", length, request.request.url);
                let upload = client
                    .post(request.request.url.clone())
                    .add_request_headers(&request.request, &None)?
                    .header("Content-Length", length)
                    .header("Content-Type", mime)
                    .body(body);
                let response = send_upload_with_idle_timeout(upload, progress_rx).await?;
                //println!("response: {}", response.status());
                let text = response.text().await?;
                let request = plugin_manager
                    .provider_upload_parse_response(text, &plugin)
                    .await
                    .map_err(|error| {
                        map_provider_error(
                            error,
                            SourcesError::Other("Unable to parse upload response".to_string()),
                        )
                    })?;

                Ok::<String, RsError>(request.source)
            } else {
                //download in temp directory if size is not available as it is necessary for upload
                let dest_source = format!(".cache/{}", format!("{}-{}", nanoid!(), filename));
                let dest = local.get_full_path(&dest_source);
                let _cleanup = StagedUploadCleanup(dest.clone());
                //println!("dest: {:?}", dest);
                PathProvider::ensure_filepath(&dest).await?;

                let mut file = File::create(&dest).await?;

                let mut writer = BufWriter::new(file);
                // Read and write chunks from the stream
                let streamreader = stream_with_idle_timeout(streamreader, None);
                tokio::pin!(streamreader);
                while let Some(chunk) = streamreader.next().await {
                    let chunk = chunk?; // Handle potential read errors
                    writer.write_all(&chunk).await?;
                }
                // Flush to ensure all data is written
                writer.flush().await?;
                writer.shutdown().await?;

                let file = File::open(&dest).await?;
                let file_size = file.metadata().await?.len();
                let stream = ReaderStream::new(file);
                let (progress_tx, progress_rx) = watch::channel(());
                let body =
                    reqwest::Body::wrap_stream(stream_with_idle_timeout(stream, Some(progress_tx)));
                let client = streaming_http_client()?;
                //println!("sending file to stream (size: {}) {}", file_size, request.request.url);
                let upload = client
                    .post(request.request.url.clone())
                    .add_request_headers(&request.request, &None)?
                    .header("Content-Length", file_size)
                    .header("Content-Type", mime)
                    .body(body);
                let response = send_upload_with_idle_timeout(upload, progress_rx).await?;
                //println!("response: {}", response.status());
                let text = response.text().await?;
                let request = plugin_manager
                    .provider_upload_parse_response(text, &plugin)
                    .await
                    .map_err(|error| {
                        map_provider_error(
                            error,
                            SourcesError::Other("Unable to parse upload response".to_string()),
                        )
                    })?;

                Ok::<String, RsError>(request.source)
            }
        })
        .map_err(|r| Error::Error("Unable to get plugin writer".to_string()));

        Ok((Box::pin(source), Box::pin(asyncwriter)))
    }

    async fn clean(&self, sources: Vec<String>) -> RsResult<Vec<(String, u64)>> {
        Ok(vec![])
    }
}

struct RsReaderStream<'a, R: AsyncRead + Unpin> {
    reader: &'a mut R,
    buf: Vec<u8>,
}

impl<'a, R: AsyncRead + Unpin> RsReaderStream<'a, R> {
    fn new(reader: &'a mut R) -> Self {
        RsReaderStream {
            reader,
            buf: vec![0; 4096], // Adjust the buffer size as needed
        }
    }
}

impl<'a, R: AsyncRead + Unpin> Stream for RsReaderStream<'a, R> {
    type Item = io::Result<Vec<u8>>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let self_mut = Pin::into_inner(self);
        let mut buf = ReadBuf::new(&mut self_mut.buf);

        match ready!(Pin::new(&mut self_mut.reader).poll_read(cx, &mut buf)) {
            Ok(()) => {
                let n = buf.filled().len();
                if n == 0 {
                    Poll::Ready(None) // EOF
                } else {
                    Poll::Ready(Some(Ok(buf.filled().to_vec())))
                }
            }
            Err(e) => Poll::Ready(Some(Err(e))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{map_provider_error, wait_for_upload_with_idle_timeout, StagedUploadCleanup};
    use crate::plugins::sources::error::SourcesError;
    use crate::Error;
    use std::time::Duration;
    use tokio::sync::watch;

    #[test]
    fn provider_upload_preserves_plugin_timeouts() {
        for (function, fallback) in [
            (
                "upload_request",
                SourcesError::NotFound(Some("large-file.zip".to_string())),
            ),
            (
                "upload_response",
                SourcesError::Other("Unable to parse upload response".to_string()),
            ),
        ] {
            let error = Error::PluginTimeout("pCloud".to_string(), function.to_string());

            assert!(matches!(
                map_provider_error(error, fallback),
                Error::PluginTimeout(plugin, timed_out_function)
                    if plugin == "pCloud" && timed_out_function == function
            ));
        }
    }

    #[test]
    fn staged_upload_cleanup_removes_partial_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("partial-upload");
        std::fs::write(&path, b"partial").unwrap();

        drop(StagedUploadCleanup(path.clone()));

        assert!(!path.exists());
    }

    #[tokio::test]
    async fn upload_watchdog_times_out_while_body_is_backpressured() {
        let (_progress_tx, progress_rx) = watch::channel(());
        let upload = std::future::pending::<crate::error::RsResult<()>>();

        let error =
            wait_for_upload_with_idle_timeout(upload, progress_rx, Duration::from_millis(10))
                .await
                .unwrap_err();

        assert!(matches!(
            error,
            Error::Io(ref error) if error.kind() == std::io::ErrorKind::TimedOut
        ));
    }
}

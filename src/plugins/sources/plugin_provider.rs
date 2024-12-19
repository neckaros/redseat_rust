use std::{io, path::PathBuf, pin::Pin, str::FromStr, sync::Arc, task::{Context, Poll}};


use axum::async_trait;
use bytes::Bytes;
use chrono::{Datelike, Utc};
use futures::{ready, AsyncReadExt, Stream, TryFutureExt, TryStreamExt};
use nanoid::nanoid;
use query_external_ip::SourceError;
use reqwest::Client;
use rs_plugin_common_interfaces::{provider::{RsProviderAddRequest, RsProviderPath}, request::RsRequest};
use tokio::{fs::{create_dir_all, remove_file, File}, io::{AsyncRead, AsyncWrite, AsyncWriteExt, BufReader, BufWriter, ReadBuf}};
use tokio_stream::StreamExt;
use tokio_util::io::ReaderStream;

use crate::{domain::{backup::Backup, library::ServerLibrary, media::MediaForUpdate, plugin::PluginWithCredential}, error::{RsError, RsResult}, model::{users::ConnectedUser, ModelController}, plugins::{sources::{path_provider::PathProvider, RsRequestHeader}, PluginManager}, routes::mw_range::RangeDefinition, server::get_server_file_path_array, Error};

use super::{error::{SourcesError, SourcesResult}, local_provider, AsyncReadPinBox, AsyncSeekableWrite, BoxedStringFuture, FileStreamResult, Source, SourceRead};

pub struct PluginProvider {
    id: String,
    plugin: PluginWithCredential,
    plugin_manager: Arc<PluginManager>,
    root: String,
}


#[async_trait]
impl Source for PluginProvider {
    async fn new(library: ServerLibrary, controller: ModelController) -> RsResult<Self> {
        let plugin_id = library.plugin.clone().ok_or(SourcesError::Other(format!("Plugin library need a plugin: {:?}", library)))?;
        let credential_id = library.credentials.clone();
        let plugin = controller.get_plugin(plugin_id, &ConnectedUser::ServerAdmin).await.map_err(|_| SourcesError::Other(format!("Plugin library need a plugin: {:?}", library)))?;
        let credential = if let Some(credential_id) = credential_id { controller.get_credential(credential_id, &ConnectedUser::ServerAdmin).await.map_err(|_| SourcesError::Other(format!("Unable to get credential: {:?}", library)))? } else { None};
        let plugin_with_credentials = PluginWithCredential { plugin, credential };

        
        Ok(Self {
            id: library.id.clone(),
            root: library.root.clone().unwrap_or("/".to_string()),
            plugin: plugin_with_credentials,
            plugin_manager: controller.plugin_manager.clone()
        })
    }

    async fn new_from_backup(backup: Backup, controller: ModelController) -> RsResult<Self> {
        let plugin_id = backup.plugin.clone().ok_or(SourcesError::Other(format!("Plugin backup need a plugin: {:?}", backup)))?;
        let credential_id = backup.credentials.clone();
        let plugin = controller.get_plugin(plugin_id, &ConnectedUser::ServerAdmin).await.map_err(|_| SourcesError::Other(format!("Plugin backup need a plugin: {:?}", backup)))?;
        let credential = if let Some(credential_id) = credential_id { controller.get_credential(credential_id, &ConnectedUser::ServerAdmin).await.map_err(|_| SourcesError::Other(format!("Unable to get credential: {:?}", backup)))? } else { None};
        let plugin_with_credentials = PluginWithCredential { plugin, credential };

        Ok(PluginProvider {
                id: backup.id.clone(),
                root: backup.path,
                plugin: plugin_with_credentials,
                plugin_manager: controller.plugin_manager.clone()
           })
   }

    async fn init(&self) -> SourcesResult<()> {
        let local = local_provider(&self.id, "PluginProvider", &Some(self.root.clone())).await.map_err(|_| SourcesError::Other("Unable to init library".to_string()))?; 

        let path_lib = local.get_full_path(".redseat");
        create_dir_all(path_lib).await?;

        let path_lib = local.get_full_path(".redseat/.thumbs");
        create_dir_all(path_lib).await?;
        let path_lib = local.get_full_path(".redseat/.portraits");
        create_dir_all(path_lib).await?;
        let path_lib = local.get_full_path(".redseat/.cache");
        create_dir_all(path_lib).await?;
        let path_lib = local.get_full_path(".redseat/.series");
        create_dir_all(path_lib).await?;
        Ok(())

    }
    
    async fn exists(&self, _source: &str) -> bool {
        true
    }
    async fn remove(&self, source: &str) -> RsResult<()> {
        self.plugin_manager.provider_remove_file(RsProviderPath { root: Some(self.root.clone()), source: source.to_string() }, &self.plugin).await
    }

    fn local_path(&self, _source: &str) -> Option<PathBuf> {
        None
    }

    async fn fill_infos(&self, source: &str, infos: &mut MediaForUpdate) -> RsResult<()> {
        let entry = self.plugin_manager.provider_info_file(RsProviderPath { root: Some(self.root.clone()), source: source.to_string() }, &self.plugin).await?;
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
    async fn get_file(&self, source: &str, _range: Option<RangeDefinition>) -> RsResult<SourceRead> {
        //println!("root: {}, source: {}", self.root, source);
        let request = self.plugin_manager.provider_get_file(RsProviderPath { root: Some(self.root.clone()), source: source.to_string() }, &self.plugin).await?;
        Ok(SourceRead::Request(request))
    }


    async fn writerseek(&self, name: &str) -> RsResult<(String, Pin<Box<dyn AsyncSeekableWrite + Send>>)> {
        Err(crate::Error::NotImplemented("Writerseek not implemented for plugin provider".to_string()))
    }

    async fn writer(&self, name: &str, length: Option<u64>, mime: Option<String>) -> RsResult<(BoxedStringFuture, Pin<Box<dyn AsyncWrite + Send>>)> {
        let (asyncwriter, asyncreader) = tokio::io::duplex(256 * 1024);
        let mut streamreader = tokio_util::io::ReaderStream::new(asyncreader);

        let request = self.plugin_manager.provider_upload_file_request(RsProviderAddRequest { root: self.root.clone(), name: name.to_string(), overwrite: false }, &self.plugin).await.map_err(|_| SourcesError::NotFound(Some(name.to_string())))?;

        let content_length = length.clone();
        let mime = mime.unwrap_or("application/octet-stream".to_string()).to_string();
        let plugin = self.plugin.clone();
        let local = local_provider(&self.id, "PluginProvider", &Some(self.root.clone())).await?; 
        let filename = name.to_string();
        let plugin_manager = self.plugin_manager.clone();
        let source = tokio::spawn(async move {

            if let Some(length) = content_length {
                let body = reqwest::Body::wrap_stream(streamreader);
                let client = Client::new();
                //println!("sending to stream (size: {}) {}", length, request.request.url);
                let response = client
                    .post(request.request.url.clone())
                    .add_request_headers(&request.request, &None)?
                    .header("Content-Length", length)
                    .header("Content-Type", mime )
                    .body(body)
                    .send()
                    .await?;
                //println!("response: {}", response.status());
                let text = response.text().await?;
                let request = plugin_manager.provider_upload_parse_response(text, &plugin).await.map_err(|_| SourcesError::Other("Unable to parse upload response".to_string()))?;

            

                Ok::<String, RsError>(request.source)
            } else { //download in temp directory if size is not available as it is necessary for upload
                let dest_source = format!(".cache/{}", format!("{}-{}", nanoid!(), filename));
                let dest = local.get_full_path(&dest_source);
                //println!("dest: {:?}", dest);
                PathProvider::ensure_filepath(&dest).await?;

                let mut file = File::create(&dest).await?;

                let mut writer = BufWriter::new(file);
                // Read and write chunks from the stream
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
                let body = reqwest::Body::wrap_stream(stream);
                let client = Client::new();
                //println!("sending file to stream (size: {}) {}", file_size, request.request.url);
                let response = client
                    .post(request.request.url.clone())
                    .add_request_headers(&request.request, &None)?
                    .header("Content-Length", file_size)
                    .header("Content-Type", mime )
                    .body(body)
                    .send()
                    .await?;
                //println!("response: {}", response.status());
                let text = response.text().await?;
                let request = plugin_manager.provider_upload_parse_response(text, &plugin).await.map_err(|_| SourcesError::Other("Unable to parse upload response".to_string()))?;

                remove_file(dest).await;

                Ok::<String, RsError>(request.source)
            }
        
            

           
        }).map_err(|r| Error::Error("Unable to get plugin writer".to_string()));
        
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

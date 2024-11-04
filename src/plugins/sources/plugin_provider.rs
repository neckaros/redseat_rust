use std::{io, path::PathBuf, pin::Pin, str::FromStr, sync::Arc, task::{Context, Poll}};


use axum::async_trait;
use bytes::Bytes;
use chrono::{Datelike, Utc};
use futures::{ready, Stream, TryFutureExt};
use query_external_ip::SourceError;
use reqwest::Client;
use rs_plugin_common_interfaces::{provider::{RsProviderAddRequest, RsProviderPath}, request::RsRequest};
use tokio::{fs::File, io::{AsyncRead, AsyncWrite, BufReader, BufWriter, ReadBuf}};
use tokio_util::io::ReaderStream;

use crate::{domain::{library::ServerLibrary, media::MediaForUpdate, plugin::PluginWithCredential}, error::RsResult, model::{users::ConnectedUser, ModelController}, plugins::PluginManager, routes::mw_range::RangeDefinition, server::get_server_file_path_array, Error};

use super::{error::{SourcesError, SourcesResult}, AsyncReadPinBox, AsyncSeekableWrite, BoxedStringFuture, FileStreamResult, Source, SourceRead};

pub struct PluginProvider {
    library: ServerLibrary,
    plugin: PluginWithCredential,
    plugin_manager: Arc<PluginManager>
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
            library,
            plugin: plugin_with_credentials,
            plugin_manager: controller.plugin_manager.clone()
        })
    }

    async fn init(&self) -> SourcesResult<()> {
        Ok(())
    }
    
    async fn exists(&self, _source: &str) -> bool {
        true
    }
    async fn remove(&self, _source: &str) -> RsResult<()> {

        Ok(())
    }

    fn local_path(&self, _source: &str) -> Option<PathBuf> {
        None
    }

    async fn fill_infos(&self, _source: &str, _infos: &mut MediaForUpdate) -> RsResult<()> {

        Ok(())
    }
    async fn get_file(&self, source: &str, _range: Option<RangeDefinition>) -> RsResult<SourceRead> {
        let request = self.plugin_manager.provider_get_file(RsProviderPath { root: self.library.root.clone(), source: source.to_string() }, &self.plugin).await.map_err(|_| SourcesError::NotFound(Some(source.to_string())))?;
        Ok(SourceRead::Request(request))
    }


    async fn writerseek(&self, name: &str) -> RsResult<(String, Pin<Box<dyn AsyncSeekableWrite + Send>>)> {
        Err(crate::Error::NotImplemented("Writer not implemented for plugin provider".to_string()))
    }

    async fn writer(&self, name: &str) -> RsResult<(BoxedStringFuture, Pin<Box<dyn AsyncWrite + Send>>)> {
        let (asyncwriter, asyncreader) = tokio::io::duplex(256 * 1024);
        let streamreader = tokio_util::io::ReaderStream::new(asyncreader);

        let request = self.plugin_manager.provider_upload_file_request(RsProviderAddRequest { root: self.library.root.clone().unwrap_or_default(), name: name.to_string(), overwrite: false }, &self.plugin).await.map_err(|_| SourcesError::NotFound(Some(name.to_string())))?;

        let source = tokio::spawn(async move {
            let body = reqwest::Body::wrap_stream(streamreader);
            let client = Client::new();
            let response = client
                .post(request.request.url)
                .body(body)
                .send()
            .await;

            "test".to_string()
        }).map_err(|r| Error::Error("Unable to get plugin writer".to_string()));
        
        Ok((Box::pin(source), Box::pin(asyncwriter)))
    }




    async fn clean(&self, sources: Vec<String>) -> RsResult<Vec<(String, u64)>> {
        Ok(vec![])
    }
    
}

impl PluginProvider {
    async fn writer<'a>(&self, name: &str) -> RsResult<(String, Pin<Box<dyn AsyncWrite + 'a>>)> {
        let (asyncwriter, asyncreader) = tokio::io::duplex(256 * 1024);
        let streamreader = tokio_util::io::ReaderStream::new(asyncreader);

        let request = self.plugin_manager.provider_upload_file_request(RsProviderAddRequest { root: self.library.root.clone().unwrap_or_default(), name: name.to_string(), overwrite: false }, &self.plugin).await.map_err(|_| SourcesError::NotFound(Some(name.to_string())))?;

        tokio::spawn(async move {
            let body = reqwest::Body::wrap_stream(streamreader);
            let client = Client::new();
            let response = client
                .post(request.request.url)
                .body(body)
                .send()
            .await;
            });
        
        Ok(("test".to_string(), Box::pin(asyncwriter)))
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

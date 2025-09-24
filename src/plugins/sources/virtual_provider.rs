use std::{future::Future, io, path::PathBuf, pin::Pin, str::FromStr, sync::Arc};


use axum::async_trait;
use bytes::Bytes;
use chrono::{Datelike, Utc};
use futures::Stream;
use query_external_ip::SourceError;
use rs_plugin_common_interfaces::request::RsRequest;
use tokio::{fs::File, io::{AsyncRead, AsyncWrite, BufReader, BufWriter}};

use crate::{domain::{backup::Backup, library::ServerLibrary, media::MediaForUpdate}, error::RsResult, model::ModelController, plugins::{sources::local_provider_for_library, PluginManager}, routes::mw_range::RangeDefinition, server::get_server_file_path_array};

use super::{error::{SourcesError, SourcesResult}, AsyncReadPinBox, AsyncSeekableWrite, BoxedStringFuture, FileStreamResult, Source, SourceRead};

pub struct VirtualProvider {
    library: ServerLibrary,
    plugin_manager: Arc<PluginManager>
}

#[async_trait]
impl Source for VirtualProvider {
    async fn new(library: ServerLibrary, controller: ModelController) -> RsResult<Self> {

        Ok(VirtualProvider {
            library,
            plugin_manager: controller.plugin_manager.clone()
        })
    }

    async fn new_from_backup(_: Backup, _: ModelController) -> RsResult<Self> {
        Err(crate::Error::NotImplemented("Virtual providers can't be used for backup".to_string()))
    }
    async fn init(&self) -> SourcesResult<()> {
        
        let source = local_provider_for_library(&self.library).await.map_err(|_| SourcesError::Other("Unable to get local provider source during init of virtual library".to_string()))?;
        source.init().await?;
       
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
        let splitted: Vec<String> = source.split('|').map(ToString::to_string).collect();
        Ok(SourceRead::Request( RsRequest {
            url: splitted.first().cloned().unwrap_or(source.to_owned()),
            selected_file: splitted.get(1).cloned(),
            ..Default::default()
        }))
    }


    async fn writer(&self, name: &str, length: Option<u64>, mime: Option<String>) -> RsResult<(BoxedStringFuture, Pin<Box<dyn AsyncWrite + Send>>)> {
        Err(crate::Error::NotImplemented("Writer not implemented for virtual provider".to_string()))
    }


    async fn writerseek(&self, name: &str) -> RsResult<(String, Pin<Box<dyn AsyncSeekableWrite + Send>>)> {
        Err(crate::Error::NotImplemented("Writer not implemented for virtual provider".to_string()))
    }


    

    async fn clean(&self, sources: Vec<String>) -> RsResult<Vec<(String, u64)>> {
        Ok(vec![])
    }
}



use std::{io, path::PathBuf, pin::Pin, str::FromStr, sync::Arc};


use axum::async_trait;
use bytes::Bytes;
use chrono::{Datelike, Utc};
use futures::Stream;
use query_external_ip::SourceError;
use rs_plugin_common_interfaces::request::RsRequest;
use tokio::{fs::File, io::{AsyncRead, AsyncWrite, BufReader, BufWriter}};

use crate::{domain::{library::ServerLibrary, media::MediaForUpdate}, model::ModelController, plugins::PluginManager, routes::mw_range::RangeDefinition, server::get_server_file_path_array};

use super::{error::{SourcesError, SourcesResult}, AsyncReadPinBox, FileStreamResult, Source, SourceRead};

pub struct VirtualProvider {
    root: PathBuf,
    library: ServerLibrary,
    plugin_manager: Arc<PluginManager>
}


#[async_trait]
impl Source for VirtualProvider {
    async fn new(library: ServerLibrary, controller: ModelController) -> SourcesResult<Self> {
        if let Some(root) = &library.root {
            Ok(VirtualProvider {
                root: PathBuf::from_str(&root).map_err(|_| SourcesError::Error)?,
                library,
                plugin_manager: controller.plugin_manager.clone()
            })
        } else {
            Err(SourcesError::Error)
        }
    }

    async fn exists(&self, _source: &str) -> bool {
        true
    }
    async fn remove(&self, _source: &str) -> SourcesResult<()> {

        Ok(())
    }

    async fn thumb(&self, _source: &str) -> SourcesResult<Vec<u8>> {
        Err(SourcesError::NotImplemented)
    }
    fn local_path(&self, _source: &str) -> Option<PathBuf> {
        None
    }

    async fn fill_infos(&self, _source: &str, _infos: &mut MediaForUpdate) -> SourcesResult<()> {

        Ok(())
    }
    async fn get_file(&self, source: &str, _range: Option<RangeDefinition>) -> SourcesResult<SourceRead> {
        let splitted: Vec<String> = source.split('|').map(ToString::to_string).collect();
        Ok(SourceRead::Request( RsRequest {
            url: splitted.first().cloned().unwrap_or(source.to_owned()),
            selected_file: splitted.get(1).cloned(),
            ..Default::default()
        }))
    }

    async fn get_file_write_stream(&self, name: &str) -> SourcesResult<(String, Pin<Box<dyn AsyncWrite + Send>>)> {
        let mut path = self.root.clone();
        let year = Utc::now().year().to_string();
        path.push(year);
        path.push(name);
        
        let file = BufWriter::new(File::create(path).await.map_err(|_| SourcesError::Error)?);
        
        Ok(("".to_string(), Box::pin(file)))
    }

    async fn write(&self, name: &str, read: Pin<Box<dyn AsyncRead + Send>>) -> SourcesResult<String> {
        Ok("test".to_owned())
    }

}



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
    library: ServerLibrary,
    plugin_manager: Arc<PluginManager>
}


#[async_trait]
impl Source for VirtualProvider {
    async fn new(library: ServerLibrary, controller: ModelController) -> SourcesResult<Self> {

        Ok(VirtualProvider {
            library,
            plugin_manager: controller.plugin_manager.clone()
        })
    }

    async fn exists(&self, _source: &str) -> bool {
        true
    }
    async fn remove(&self, _source: &str) -> SourcesResult<()> {

        Ok(())
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


    async fn write<'a>(&self, _name: &str, _read: Pin<Box<dyn AsyncRead + Send + 'a>>) -> SourcesResult<String> {
        Ok("test".to_owned())
    }

}



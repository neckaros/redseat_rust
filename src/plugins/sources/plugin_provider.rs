use std::{io, path::PathBuf, pin::Pin, str::FromStr, sync::Arc};


use axum::async_trait;
use bytes::Bytes;
use chrono::{Datelike, Utc};
use futures::Stream;
use query_external_ip::SourceError;
use rs_plugin_common_interfaces::{provider::RsProviderPath, request::RsRequest};
use tokio::{fs::File, io::{AsyncRead, AsyncWrite, BufReader, BufWriter}};

use crate::{domain::{library::ServerLibrary, media::MediaForUpdate, plugin::PluginWithCredential}, error::RsResult, model::{users::ConnectedUser, ModelController}, plugins::PluginManager, routes::mw_range::RangeDefinition, server::get_server_file_path_array};

use super::{error::{SourcesError, SourcesResult}, AsyncReadPinBox, FileStreamResult, Source, SourceRead};

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


    async fn write<'a>(&self, _name: &str, _read: Pin<Box<dyn AsyncRead + Send + 'a>>) -> RsResult<String> {
        Ok("test".to_owned())
    }

}



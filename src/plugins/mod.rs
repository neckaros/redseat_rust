use std::{fs::read_dir, path::PathBuf, sync::Mutex};

use extism::{convert::Json, Manifest, PluginBuilder, Wasm};
use rs_plugin_common_interfaces::{PluginInformation, PluginType};

use extism::Plugin as ExtismPlugin;

use crate::{domain::{library::ServerLibrary, plugin::{self, PluginWasm}, rs_link::RsLink}, model::ModelController, server::get_server_folder_path_array, Result};

use self::sources::{error::SourcesResult, path_provider::PathProvider, virtual_provider::VirtualProvider, Source};

pub mod sources;
pub mod error;
pub mod medias;


pub struct PluginManager {
    pub plugins: Vec<PluginWasm>
}


pub async fn get_plugin_fodler() -> crate::Result<PathBuf> {
    get_server_folder_path_array(vec!["plugins"]).await
}

pub async fn list_plugins() -> crate::Result<impl Iterator<Item = PluginWasm>> {
    let folder = get_plugin_fodler().await?;
    Ok(std::fs::read_dir(folder)?
        // Filter out all those directory entries which couldn't be read
        .filter_map(|res| res.ok())
        // Map the directory entries to paths
        .map(|dir_entry| dir_entry.path())
        // Filter out all paths with extensions other than `csv`
        .filter_map(|path| {
            if path.extension().map_or(false, |ext| ext == "wasm") {
                Some(path)
            } else {
                None
            }
        })
        .filter_map(|path| {
            let manifest = Manifest::new([path.clone()]);
            let plugin = PluginBuilder::new(manifest)
                .with_wasi(true)
                .build();
            if let Ok(mut plugin) = plugin {
                if let Ok(Json(res)) = plugin.call::<&str, Json<PluginInformation>>("infos", "") {
                    let p = PluginWasm {
                        path,
                        infos: res,
                        plugin:Mutex::new(plugin),
                    };
                    Some(p)
                } else {
                    None
                }
            } else {
                None
            }
                
        }))
}


/*pub fn parse_url_plugin(url: String, plugin: PluginInformation) {
    let manifest = Manifest::new([plugin.]);
    let plugin = PluginBuilder::new(manifest)
        .with_wasi(true)
        .build()?;
        let Json(res) = plugin.call::<&str, Json<PluginInformation>>("infos", "")?;
}*/

pub mod url;

impl PluginManager {
    pub async fn new() -> Result<Self> {
        let plugins: Vec<PluginWasm> = list_plugins().await?.collect();
        Ok(
            PluginManager { plugins }
        )
    }

    pub async fn source_for_library(&self, library: ServerLibrary, controller: ModelController) -> SourcesResult<Box<dyn Source>> {
        let source: Box<dyn Source> = if library.source == "PathProvider" {
            let source = PathProvider::new(library, controller).await?;
            Box::new(source)
        } else {
            let source = VirtualProvider::new(library, controller).await?;
            Box::new(source)
        };
        Ok(source)
    }

}
use std::{fs::read_dir, path::PathBuf, sync::Mutex};

use extism::{convert::Json, Manifest, PluginBuilder, Wasm};
use rs_plugin_common_interfaces::{PluginInformation, PluginType};

use extism::Plugin as ExtismPlugin;
use tokio::{fs::File, io::AsyncReadExt, sync::RwLock};

use crate::{domain::{library::ServerLibrary, plugin::{self, PluginWasm}}, error::RsResult, model::ModelController, server::get_server_folder_path_array, tools::log::{log_error, log_info, LogServiceType}, Error, Result};

use self::sources::{error::SourcesResult, path_provider::PathProvider, virtual_provider::VirtualProvider, Source};

pub mod sources;
pub mod error;
pub mod medias;


pub struct PluginManager {
    pub plugins: RwLock<Vec<PluginWasm>>
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
            let manifest = Manifest::new([path.clone()]).with_allowed_host("*");
            let plugin = PluginBuilder::new(manifest)
                .with_wasi(true)
                .build();

            if let Ok(mut plugin) = plugin {
                let infos = plugin.call::<&str, Json<PluginInformation>>("infos", "");
                if let Ok(Json(res)) = infos {
                    let filename = path.file_name().unwrap().to_str().unwrap();
                    log_info(crate::tools::log::LogServiceType::Plugin, format!("Loaded plugin {} ({:?}) -> {:?}", res.name, res.capabilities, path));
                    let p = PluginWasm {
                        filename: filename.to_string(),
                        path,
                        infos: res,
                        plugin:Mutex::new(plugin),
                    };
                    Some(p)
                } else {
                    log_error(crate::tools::log::LogServiceType::Other, format!("Error getting plugin informations: {:?} {:?}", &path, infos.err()));
                    None
                }
            } else {
                log_error(crate::tools::log::LogServiceType::Other, format!("Error loading plugin: {:?}", &path));
                None
            }
                
        }))

       
}

pub async fn list_other_plugins() -> crate::Result<Vec<PluginInformation>> {
    let folder = get_plugin_fodler().await?;
    let files = std::fs::read_dir(folder)?
        // Filter out all those directory entries which couldn't be read
        .filter_map(|res| res.ok())
        // Map the directory entries to paths
        .map(|dir_entry| dir_entry.path())
        // Filter out all paths with extensions other than `csv`
        .filter_map(|path| {
            if path.extension().map_or(false, |ext| ext == "rsplugin") {
                Some(path)
            } else {
                None
            }
        });
    
    let mut plugins = vec![];
    for path in files.into_iter() {
        let mut file = File::open(path).await?;
  
        let mut manifest_string = String::new();
        file.read_to_string(&mut manifest_string).await?;
        let info: PluginInformation = serde_json::from_str(&manifest_string)?;
        plugins.push(info);
        
      
    }

    Ok(plugins)

       
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
        Ok(
            PluginManager { plugins: RwLock::new(vec![]) }
        )
    }

    pub async fn reload(&self) -> Result<()> {
        let mut plugins: Vec<PluginWasm> = list_plugins().await?.collect();
        self.plugins.write().await.append(&mut plugins);
        Ok(())
    }


    pub async fn load_wasm_plugin(&self, filename: &str) -> RsResult<PluginInformation> {
        let mut folder = get_plugin_fodler().await?;
        folder.push(filename);
        let existing = self.plugins.read().await.iter().position(|e| e.path == folder);
        if let Some(existing) = existing {
            self.plugins.write().await.swap_remove(existing);
        }
        let manifest = Manifest::new([folder.clone()]).with_allowed_host("*");
        let mut plugin = PluginBuilder::new(manifest)
            .with_wasi(true)
            .build()?;
    
        let Json(infos) = plugin.call::<&str, Json<PluginInformation>>("infos", "")?;
       
        let filename = folder.file_name().unwrap().to_str().unwrap();
        log_info(crate::tools::log::LogServiceType::Plugin, format!("Loaded plugin {} ({:?}) -> {:?}", infos.name, infos.capabilities, folder));
        let p = PluginWasm {
            filename: filename.to_string(),
            path: folder,
            infos: infos.clone(),
            plugin:Mutex::new(plugin),
        };
        self.plugins.write().await.push(p);
        Ok(infos)
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
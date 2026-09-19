use std::{
    fs::read_dir,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use extism::{convert::Json, FromBytesOwned, Manifest, PluginBuilder, ToBytes, Wasm};
use rs_plugin_common_interfaces::{PluginInformation, PluginType, RsRequest};

use extism::Plugin as ExtismPlugin;
use sources::plugin_provider::PluginProvider;
use tokio::{
    fs::File,
    io::AsyncReadExt,
    sync::{Mutex, RwLock},
};

use crate::{
    domain::{
        backup::Backup,
        library::ServerLibrary,
        plugin::{self, PluginWasm},
    },
    error::RsResult,
    model::ModelController,
    server::get_server_folder_path_array,
    tools::log::{log_error, log_info, LogServiceType},
    Error, Result,
};

use self::sources::{
    error::SourcesResult, path_provider::PathProvider, virtual_provider::VirtualProvider, Source,
};

pub mod error;
pub mod medias;
pub mod sources;
pub mod token;

pub use url::PluginTarget;

// Plugin exports resolve metadata and transfer URLs; media bytes are streamed separately.
const PLUGIN_CALL_TIMEOUT: Duration = Duration::from_secs(30);
const PLUGIN_SLOW_CALL_THRESHOLD: Duration = Duration::from_secs(5);

impl PluginWasm {
    pub async fn call_get_error_code<I, O>(
        &self,
        function: &'static str,
        input: I,
    ) -> std::result::Result<O, (extism::Error, i32)>
    where
        I: for<'a> ToBytes<'a> + Send + 'static,
        O: FromBytesOwned + Send + 'static,
    {
        let plugin = self.plugin.clone();
        let plugin_name = self.infos.name.clone();
        let queued_at = Instant::now();
        let mut plugin = match tokio::time::timeout(PLUGIN_CALL_TIMEOUT, plugin.lock_owned()).await {
            Ok(plugin) => plugin,
            Err(_) => {
                log_error(
                    LogServiceType::Plugin,
                    format!(
                        "Plugin {plugin_name} call {function} timed out waiting for the plugin"
                    ),
                );
                return Err((extism::Error::msg("timeout"), 504));
            }
        };
        let queued_for = queued_at.elapsed();
        let started = Instant::now();
        let result = tokio::task::spawn_blocking(move || {
            plugin.call_get_error_code::<I, O>(function, input)
        })
        .await
        .unwrap_or_else(|error| Err((extism::Error::msg(error.to_string()), 500)));
        let elapsed = started.elapsed();
        if let Err((error, _)) = &result {
            log_error(
                LogServiceType::Plugin,
                format!(
                    "Plugin {plugin_name} call {function} failed after {elapsed:?}: {error:?}"
                ),
            );
        } else if elapsed >= PLUGIN_SLOW_CALL_THRESHOLD {
            log_info(
                LogServiceType::Plugin,
                format!(
                    "Plugin {plugin_name} call {function} completed in {elapsed:?} after waiting {queued_for:?}"
                ),
            );
        }
        result
    }

    pub async fn call<I, O>(
        &self,
        function: &'static str,
        input: I,
    ) -> std::result::Result<O, extism::Error>
    where
        I: for<'a> ToBytes<'a> + Send + 'static,
        O: FromBytesOwned + Send + 'static,
    {
        self.call_get_error_code(function, input)
            .await
            .map_err(|(error, _)| error)
    }
}

pub fn plugin_call_error(
    plugin: &str,
    function: &str,
    error: (extism::Error, i32),
) -> Error {
    if error.0.root_cause().to_string() == "timeout" {
        Error::PluginTimeout(plugin.to_string(), function.to_string())
    } else {
        Error::PluginError(error.1, error.0.to_string())
    }
}

pub struct PluginManager {
    pub plugins: RwLock<Vec<PluginWasm>>,
}

pub async fn get_plugin_fodler() -> crate::Result<PathBuf> {
    get_server_folder_path_array(vec!["plugins"]).await
}

pub async fn list_plugins() -> crate::Result<Vec<PluginWasm>> {
    let folder = get_plugin_fodler().await?;
    log_info(
        crate::tools::log::LogServiceType::Plugin,
        format!("Loaded plugins from local path -> {:?}", folder),
    );
    tokio::task::spawn_blocking(move || {
        Ok(std::fs::read_dir(folder)?
            .filter_map(|res| res.ok())
            .map(|dir_entry| dir_entry.path())
            .filter(|path| path.extension().map_or(false, |ext| ext == "wasm"))
            .filter_map(|path| {
                let manifest = Manifest::new([path.clone()])
                    .with_allowed_host("*")
                    .with_timeout(PLUGIN_CALL_TIMEOUT);
                let plugin = PluginBuilder::new(manifest)
                    .with_wasi(true)
                    .with_http_response_headers(true)
                    .build();

                match plugin {
                    Ok(mut plugin) => {
                        let infos = plugin.call::<&str, Json<PluginInformation>>("infos", "");
                        if let Ok(Json(res)) = infos {
                            if let Some(filename) = path.file_name() {
                                log_info(
                                    crate::tools::log::LogServiceType::Plugin,
                                    format!(
                                        "Loaded plugin {} ({:?}) -> {:?}",
                                        res.name, res.capabilities, path
                                    ),
                                );
                                Some(PluginWasm {
                                    filename: filename.to_str().unwrap().to_string(),
                                    path,
                                    infos: res,
                                    plugin: Arc::new(Mutex::new(plugin)),
                                })
                            } else {
                                log_error(
                                    crate::tools::log::LogServiceType::Other,
                                    format!("Error getting plugin informations: {:?}", &path),
                                );
                                None
                            }
                        } else {
                            log_error(
                                crate::tools::log::LogServiceType::Other,
                                format!(
                                    "Error getting plugin informations: {:?} {:?}",
                                    &path,
                                    infos.err()
                                ),
                            );
                            None
                        }
                    }
                    Err(err) => {
                        log_error(
                            crate::tools::log::LogServiceType::Other,
                            format!("Error loading plugin: {:?} {:?}", &path, err),
                        );
                        None
                    }
                }
            })
            .collect())
    })
    .await?
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

pub mod provider;
pub mod url;

impl PluginManager {
    pub async fn new() -> Result<Self> {
        Ok(PluginManager {
            plugins: RwLock::new(vec![]),
        })
    }

    pub(crate) async fn plugin_by_filename(&self, filename: &str) -> Option<PluginWasm> {
        self.plugins
            .read()
            .await
            .iter()
            .find(|plugin| plugin.filename == filename)
            .cloned()
    }

    pub async fn reload(&self) -> Result<()> {
        let mut plugins = list_plugins().await?;
        log_info(
            LogServiceType::Plugin,
            format!("Reloaded {} plugins", plugins.len()),
        );
        self.plugins.write().await.clear();
        self.plugins.write().await.append(&mut plugins);
        Ok(())
    }

    pub async fn load_wasm_plugin(&self, filename: &str) -> RsResult<PluginInformation> {
        let mut folder = get_plugin_fodler().await?;
        folder.push(filename);
        let existing = self
            .plugins
            .read()
            .await
            .iter()
            .position(|e| e.path == folder);
        if let Some(existing) = existing {
            self.plugins.write().await.swap_remove(existing);
        }
        let plugin_path = folder.clone();
        let (plugin, infos) = tokio::task::spawn_blocking(move || {
            let manifest = Manifest::new([plugin_path])
                .with_allowed_host("*")
                .with_timeout(PLUGIN_CALL_TIMEOUT);
            let mut plugin = PluginBuilder::new(manifest)
                .with_wasi(true)
                .with_http_response_headers(true)
                .build()?;
            let Json(infos) = plugin.call::<&str, Json<PluginInformation>>("infos", "")?;
            Ok::<_, Error>((plugin, infos))
        })
        .await??;

        let filename = folder.file_name().unwrap().to_str().unwrap();
        log_info(
            crate::tools::log::LogServiceType::Plugin,
            format!(
                "Loaded wasm plugin {} ({:?}) -> {:?}",
                infos.name, infos.capabilities, folder
            ),
        );
        let p = PluginWasm {
            filename: filename.to_string(),
            path: folder,
            infos: infos.clone(),
            plugin: Arc::new(Mutex::new(plugin)),
        };
        self.plugins.write().await.push(p);
        Ok(infos)
    }

    pub async fn source_for_library(
        &self,
        library: ServerLibrary,
        controller: ModelController,
    ) -> RsResult<Box<dyn Source>> {
        let source: Box<dyn Source> = if library.source == "PathProvider" {
            let source = PathProvider::new(library, controller).await?;
            Box::new(source)
        } else if library.source == "PluginProvider" {
            let source = PluginProvider::new(library, controller).await?;
            Box::new(source)
        } else {
            let source = VirtualProvider::new(library, controller).await?;
            Box::new(source)
        };
        Ok(source)
    }

    pub async fn provider_for_backup(
        &self,
        backup: Backup,
        controller: ModelController,
    ) -> RsResult<Box<dyn Source>> {
        let source: Box<dyn Source> = if backup.source == "PathProvider" {
            let source = PathProvider::new_from_backup(backup, controller).await?;
            Box::new(source)
        } else if backup.source == "PluginProvider" {
            let source = PluginProvider::new_from_backup(backup, controller).await?;
            Box::new(source)
        } else {
            let source = VirtualProvider::new_from_backup(backup, controller).await?;
            Box::new(source)
        };
        Ok(source)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // (module (func (export "spin") (loop (br 0))))
    const SPIN_WASM: &[u8] = &[
        0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, 0x01, 0x04, 0x01, 0x60, 0x00, 0x00,
        0x03, 0x02, 0x01, 0x00, 0x07, 0x08, 0x01, 0x04, 0x73, 0x70, 0x69, 0x6e, 0x00, 0x00,
        0x0a, 0x09, 0x01, 0x07, 0x00, 0x03, 0x40, 0x0c, 0x00, 0x0b, 0x0b,
    ];

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn blocking_plugin_call_does_not_starve_async_tasks() {
        let manifest = Manifest::new([Wasm::data(SPIN_WASM)])
            .with_timeout(Duration::from_millis(100));
        let plugin = PluginBuilder::new(manifest).build().unwrap();
        let mut infos = PluginInformation::default();
        infos.name = "slow-test-plugin".to_string();
        let plugin = PluginWasm {
            filename: "slow-test-plugin.wasm".to_string(),
            path: PathBuf::from("slow-test-plugin.wasm"),
            infos,
            plugin: Arc::new(Mutex::new(plugin)),
        };

        let second_plugin = plugin.clone();
        let first_call = tokio::spawn(async move { plugin.call::<(), ()>("spin", ()).await });
        let second_call =
            tokio::spawn(async move { second_plugin.call::<(), ()>("spin", ()).await });
        tokio::time::timeout(
            Duration::from_millis(50),
            tokio::time::sleep(Duration::from_millis(5)),
        )
        .await
        .expect("an unrelated async task should remain schedulable");

        for call in [first_call, second_call] {
            let error = call.await.unwrap().unwrap_err();
            assert_eq!(error.root_cause().to_string(), "timeout");
        }
    }

    #[test]
    fn extism_timeout_is_mapped_to_gateway_timeout() {
        assert!(matches!(
            plugin_call_error("pcloud", "download_request", (extism::Error::msg("timeout"), 0)),
            Error::PluginTimeout(plugin, function)
                if plugin == "pcloud" && function == "download_request"
        ));
    }
}

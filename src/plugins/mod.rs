use crate::domain::library::ServerLibrary;

use self::sources::{error::SourcesResult, path_provider::PathProvider, virtual_provider::VirtualProvider, Source};

pub mod sources;
pub mod error;



#[derive(Clone)]
pub struct PluginManager {

}

impl PluginManager {
    pub fn new() -> Self {
        PluginManager {  }
    }

    pub async fn source_for_library(&self, library: ServerLibrary) -> SourcesResult<Box<dyn Source>> {
        let source: Box<dyn Source> = if library.source == "PathProvider" {
            let source = PathProvider::new(library).await?;
            Box::new(source)
        } else {
            let source = VirtualProvider::new(library).await?;
            Box::new(source)
        };
        Ok(source)
    }
}
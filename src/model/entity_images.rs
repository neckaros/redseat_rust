use rs_plugin_common_interfaces::{
    domain::rs_ids::RsIds,
    lookup::RsLookupQuery,
    ExternalImage, ImageType, RsRequest,
};
use tokio::io::AsyncWriteExt;

use crate::{
    error::RsResult,
    plugins::sources::{AsyncReadPinBox, FileStreamResult, Source, SourceRead},
    tools::{
        image_tools::{resize_image_reader, ImageSize},
        log::{log_error, log_info, LogServiceType},
    },
};

use super::{error::Error, users::ConnectedUser, ModelController};

/// Configuration for entity image operations.
/// Each entity type (movie, serie, book, person) constructs one of these.
pub struct EntityImageConfig<'a> {
    /// Library subfolder for stored images, e.g. ".movies", ".series", ".books", ".portraits"
    pub folder: &'a str,
    /// Cache key prefix, e.g. "movie", "serie", "book", "person"
    pub cache_prefix: &'a str,
}

impl ModelController {
    /// Unified image URL selection from a list of ExternalImages.
    /// Prefers exact match on kind, falls back to first exact-match image.
    pub fn select_image_url(images: Vec<ExternalImage>, kind: &ImageType) -> Option<RsRequest> {
        let exact_images: Vec<_> = images
            .into_iter()
            .filter(|image| image.match_type.is_some())
            .collect();
        let first_kind_match = exact_images
            .iter()
            .find(|image| image.kind.as_ref() == Some(kind))
            .map(|image| image.url.clone());
        first_kind_match.or_else(|| exact_images.into_iter().next().map(|image| image.url))
    }

    /// Fetch images for any entity type via plugins.
    pub async fn get_entity_images(
        &self,
        lookup_query: RsLookupQuery,
        library_id: Option<String>,
        requesting_user: &ConnectedUser,
    ) -> RsResult<Vec<ExternalImage>> {
        match self
            .exec_lookup_images(lookup_query, library_id, requesting_user, None)
            .await
        {
            Ok(images) => Ok(images),
            Err(error) => {
                log_error(
                    LogServiceType::Plugin,
                    format!("entity image lookup failed: {:#}", error),
                );
                Ok(Vec::new())
            }
        }
    }

    /// Get the best image URL for any entity type.
    pub async fn get_entity_image_url(
        &self,
        lookup_query: RsLookupQuery,
        library_id: Option<String>,
        kind: &ImageType,
        requesting_user: &ConnectedUser,
    ) -> RsResult<Option<RsRequest>> {
        let images = self
            .get_entity_images(lookup_query, library_id, requesting_user)
            .await?;
        Ok(Self::select_image_url(images, kind))
    }

    /// Download an entity image as an async reader.
    pub async fn download_entity_image(
        &self,
        lookup_query: RsLookupQuery,
        library_id: Option<String>,
        kind: &ImageType,
        requesting_user: &ConnectedUser,
    ) -> crate::Result<AsyncReadPinBox> {
        let request = self
            .get_entity_image_url(lookup_query, library_id.clone(), kind, requesting_user)
            .await?
            .ok_or(crate::Error::NotFound(format!(
                "Unable to get image url for kind: {:?}",
                kind
            )))?;
        let reader = SourceRead::Request(request)
            .into_reader(
                library_id.as_deref(),
                None,
                None,
                Some((self.clone(), requesting_user)),
                None,
            )
            .await?;
        Ok(reader.stream)
    }

    /// Serve a cached image for an external ID that is not in the local DB.
    /// Checks cache, fetches if missing, and returns the cached file.
    pub async fn serve_cached_entity_image(
        &self,
        library_id: &str,
        external_id: &str,
        lookup_query: RsLookupQuery,
        kind: &ImageType,
        config: &EntityImageConfig<'_>,
        requesting_user: &ConnectedUser,
    ) -> crate::Result<FileStreamResult<AsyncReadPinBox>> {
        let local_provider = self.library_source_for_library(library_id).await?;
        let image_path = format!(
            "cache/{}-{}-{}.avif",
            config.cache_prefix,
            external_id.replace(':', "-"),
            kind,
        );

        if !local_provider.exists(&image_path).await {
            let image_request = self
                .get_entity_image_url(
                    lookup_query,
                    Some(library_id.to_string()),
                    kind,
                    requesting_user,
                )
                .await?
                .ok_or(crate::Error::NotFound(format!(
                    "Unable to get {} image url: {} kind {:?}",
                    config.cache_prefix, external_id, kind,
                )))?;
            let (_, mut writer) = local_provider.get_file_write_stream(&image_path).await?;
            let image_reader = SourceRead::Request(image_request)
                .into_reader(
                    Some(library_id),
                    None,
                    None,
                    Some((self.clone(), requesting_user)),
                    None,
                )
                .await?;
            let resized = resize_image_reader(
                image_reader.stream,
                ImageSize::Large.to_size(),
                image::ImageFormat::Avif,
                Some(70),
                false,
            )
            .await?;
            writer.write_all(&resized).await?;
        }

        let source = local_provider.get_file(&image_path, None).await?;
        match source {
            SourceRead::Stream(s) => Ok(s),
            SourceRead::Request(_) => Err(crate::Error::GenericRedseatError),
        }
    }

    /// Serve a local entity image (non-external-ID path).
    /// Checks if image exists, refreshes if not, then serves.
    pub async fn serve_local_entity_image<F>(
        &self,
        library_id: &str,
        entity_id: &str,
        kind: &ImageType,
        size: Option<ImageSize>,
        config: &EntityImageConfig<'_>,
        requesting_user: &ConnectedUser,
        refresh_fn: F,
    ) -> crate::Result<FileStreamResult<AsyncReadPinBox>>
    where
        F: std::future::Future<Output = RsResult<()>>,
    {
        if !self
            .has_library_image(
                library_id,
                config.folder,
                entity_id,
                Some(kind.clone()),
                requesting_user,
            )
            .await?
        {
            log_info(
                LogServiceType::Source,
                format!("Updating {} image: {}", config.cache_prefix, entity_id),
            );
            if let Err(e) = refresh_fn.await {
                log_error(
                    LogServiceType::Source,
                    format!("Failed to refresh {} image {}: {:#}", config.cache_prefix, entity_id, e),
                );
            }
        }

        self.library_image(
            library_id,
            config.folder,
            entity_id,
            Some(kind.clone()),
            size,
            requesting_user,
        )
        .await
    }
}

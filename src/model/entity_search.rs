use rs_plugin_common_interfaces::lookup::{
    RsLookupMetadataResultWrapper, RsLookupMetadataResults, RsLookupQuery,
};

use crate::{domain::library::LibraryRole, error::RsResult};

use super::{users::ConnectedUser, ModelController};

/// Type alias for grouped search results (source_id, source_name, results)
pub type SearchResultGroups = Vec<(String, String, RsLookupMetadataResults)>;

impl ModelController {
    /// Generic search that queries Trakt (pre-fetched) + plugins, filters results by type.
    ///
    /// `trakt_entries`: Pre-fetched and wrapped Trakt results (caller builds the correct variant).
    /// `result_filter`: Predicate to filter plugin results by entity type.
    pub async fn search_entity(
        &self,
        library_id: &str,
        lookup_query: RsLookupQuery,
        result_filter: fn(&RsLookupMetadataResultWrapper) -> bool,
        trakt_entries: Option<Vec<RsLookupMetadataResultWrapper>>,
        sources: Option<Vec<String>>,
        requesting_user: &ConnectedUser,
    ) -> RsResult<SearchResultGroups> {
        requesting_user.check_library_role(library_id, LibraryRole::Read)?;
        let mut groups: SearchResultGroups = Vec::new();

        if let Some(entries) = trakt_entries {
            if !entries.is_empty() {
                groups.push((
                    "trakt".to_string(),
                    "trakt".to_string(),
                    RsLookupMetadataResults {
                        results: entries,
                        next_page_key: None,
                    },
                ));
            }
        }

        let plugin_results = self
            .exec_lookup_metadata_grouped(
                lookup_query,
                Some(library_id.to_string()),
                requesting_user,
                None,
                sources.as_deref(),
            )
            .await?;

        for (id, name, RsLookupMetadataResults { results, next_page_key }) in plugin_results {
            let filtered: Vec<_> = results.into_iter().filter(|r| result_filter(r)).collect();
            if !filtered.is_empty() {
                groups.push((
                    id,
                    name,
                    RsLookupMetadataResults {
                        results: filtered,
                        next_page_key,
                    },
                ));
            }
        }

        Ok(groups)
    }

    /// Generic streaming search. Same as search_entity but returns an mpsc receiver.
    pub async fn search_entity_stream(
        &self,
        library_id: &str,
        lookup_query: RsLookupQuery,
        result_filter: fn(&RsLookupMetadataResultWrapper) -> bool,
        trakt_entries: Option<Vec<RsLookupMetadataResultWrapper>>,
        sources: Option<Vec<String>>,
        requesting_user: &ConnectedUser,
    ) -> RsResult<tokio::sync::mpsc::Receiver<(String, String, RsLookupMetadataResults)>> {
        requesting_user.check_library_role(library_id, LibraryRole::Read)?;
        let (tx, rx) = tokio::sync::mpsc::channel(16);

        if let Some(entries) = trakt_entries {
            if !entries.is_empty() {
                let _ = tx
                    .send((
                        "trakt".to_string(),
                        "trakt".to_string(),
                        RsLookupMetadataResults {
                            results: entries,
                            next_page_key: None,
                        },
                    ))
                    .await;
            }
        }

        let mut plugin_rx = self
            .exec_lookup_metadata_stream_grouped(
                lookup_query,
                Some(library_id.to_string()),
                requesting_user,
                None,
                sources.as_deref(),
            )
            .await?;

        tokio::spawn(async move {
            while let Some((id, name, entries)) = plugin_rx.recv().await {
                let RsLookupMetadataResults {
                    results,
                    next_page_key,
                } = entries;
                let filtered: Vec<_> = results.into_iter().filter(|r| result_filter(r)).collect();
                if !filtered.is_empty() {
                    if tx
                        .send((
                            id,
                            name,
                            RsLookupMetadataResults {
                                results: filtered,
                                next_page_key,
                            },
                        ))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
            }
        });

        Ok(rx)
    }
}

use std::collections::HashMap;

use rs_plugin_common_interfaces::{
    domain::rs_ids::RsIds,
    lookup::{RsLookupMetadataResultWrapper, RsLookupMetadataResults, RsLookupQuery},
};

use crate::{domain::library::LibraryRole, error::RsResult};

use super::{users::ConnectedUser, ModelController};

/// Type alias for grouped search results (source_id, source_name, results)
pub type SearchResultGroups = Vec<(String, String, RsLookupMetadataResults)>;

/// Merge RsIds across results that share at least one common ID.
/// Uses union-find for O(n·k) performance where n = total results, k = IDs per result.
pub fn merge_result_ids(groups: &mut SearchResultGroups) {
    // 1. Flatten all results into a linear index
    let mut entries: Vec<(usize, usize)> = Vec::new();
    let mut ids_vec: Vec<RsIds> = Vec::new();
    for (gi, (_, _, data)) in groups.iter().enumerate() {
        for (ri, wrapper) in data.results.iter().enumerate() {
            let extracted = wrapper.metadata.extract_ids().unwrap_or_default();
            entries.push((gi, ri));
            ids_vec.push(extracted);
        }
    }

    let n = entries.len();
    if n <= 1 {
        return;
    }

    // 2. Build index: "key:value" → set of flat indices
    let mut id_to_indices: HashMap<String, Vec<usize>> = HashMap::new();
    for (idx, ids) in ids_vec.iter().enumerate() {
        for id_str in ids.as_all_ids() {
            id_to_indices.entry(id_str).or_default().push(idx);
        }
    }

    // 3. Union-Find with path halving
    let mut parent: Vec<usize> = (0..n).collect();
    fn find(parent: &mut [usize], mut x: usize) -> usize {
        while parent[x] != x {
            parent[x] = parent[parent[x]];
            x = parent[x];
        }
        x
    }
    for indices in id_to_indices.values() {
        if indices.len() > 1 {
            let root = indices[0];
            for &idx in &indices[1..] {
                let ra = find(&mut parent, root);
                let rb = find(&mut parent, idx);
                if ra != rb {
                    parent[ra] = rb;
                }
            }
        }
    }

    // 4. Group by root and merge
    let mut components: HashMap<usize, Vec<usize>> = HashMap::new();
    for i in 0..n {
        components
            .entry(find(&mut parent, i))
            .or_default()
            .push(i);
    }

    for members in components.values() {
        if members.len() <= 1 {
            continue;
        }
        let mut merged = RsIds::default();
        for &idx in members {
            merged.merge(&ids_vec[idx]);
        }
        for &idx in members {
            let (gi, ri) = entries[idx];
            groups[gi].2.results[ri].metadata.apply_ids(&merged);
        }
    }
}

/// Enrich results with IDs from previously seen results (for streaming).
/// Returns extracted IDs from the new results to be added to the seen set.
fn enrich_from_seen(
    results: &mut [RsLookupMetadataResultWrapper],
    seen_ids: &[RsIds],
) -> Vec<RsIds> {
    let mut new_ids = Vec::new();
    for result in results.iter_mut() {
        if let Some(mut result_ids) = result.metadata.extract_ids() {
            let mut enriched = false;
            for seen in seen_ids {
                if result_ids.has_common_id(seen) {
                    result_ids.merge(seen);
                    enriched = true;
                }
            }
            if enriched {
                result.metadata.apply_ids(&result_ids);
            }
            new_ids.push(result_ids);
        }
    }
    new_ids
}

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

        merge_result_ids(&mut groups);
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

        // Collect Trakt IDs for streaming enrichment
        let mut seen_ids: Vec<RsIds> = Vec::new();

        if let Some(entries) = trakt_entries {
            if !entries.is_empty() {
                for e in &entries {
                    if let Some(ids) = e.metadata.extract_ids() {
                        seen_ids.push(ids);
                    }
                }
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
                let mut filtered: Vec<_> =
                    results.into_iter().filter(|r| result_filter(r)).collect();
                if !filtered.is_empty() {
                    let new_ids = enrich_from_seen(&mut filtered, &seen_ids);
                    seen_ids.extend(new_ids);
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

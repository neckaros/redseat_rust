use std::collections::HashMap;

use rs_plugin_common_interfaces::domain::{
    media::MediaItemReference, other_ids::OtherIds, Relations,
};

use super::{
    tags::{TagForAdd, TagQuery},
    users::ConnectedUser,
    ModelController,
};
use crate::error::RsResult;

fn tag_external_ids(otherids: Option<OtherIds>, source_id: &str) -> Option<OtherIds> {
    let mut otherids = otherids.unwrap_or_default();
    if !source_id.trim().is_empty() && !otherids.as_slice().iter().any(|id| id == source_id) {
        otherids.0.push(source_id.to_owned());
    }
    (!otherids.as_slice().is_empty()).then_some(otherids)
}

impl ModelController {
    /// Resolve a plugin tag snapshot to library-local tag references.
    ///
    /// `None` means the provider did not return tag information. A present empty
    /// snapshot is preserved as `Some(Vec::new())` so callers can distinguish it.
    pub(crate) async fn resolve_refresh_tags(
        &self,
        library_id: &str,
        relations: &Relations,
        requesting_user: &ConnectedUser,
    ) -> RsResult<Option<Vec<MediaItemReference>>> {
        if relations.tags.is_none() && relations.tags_details.is_none() {
            return Ok(None);
        }

        let mut resolved: Vec<MediaItemReference> = Vec::new();
        let mut provider_to_local = HashMap::new();
        if let Some(tags_details) = &relations.tags_details {
            let mut pending: Vec<_> = tags_details.iter().collect();
            for tag in tags_details {
                let otherids = tag_external_ids(tag.otherids.clone(), &tag.id);
                if let Some(reference) = self
                    .get_tag_by_external_id(
                        library_id,
                        &tag.id,
                        Vec::new(),
                        otherids,
                        requesting_user,
                    )
                    .await?
                {
                    provider_to_local.insert(tag.id.clone(), reference.id.clone());
                    push_unique_tag(&mut resolved, reference);
                    pending.retain(|candidate| candidate.id != tag.id);
                }
            }

            while !pending.is_empty() {
                let mut progressed = false;
                let mut index = 0;
                while index < pending.len() {
                    let tag = pending[index];
                    let parent = if let Some(provider_parent) = &tag.parent {
                        if let Some(local_parent) = provider_to_local.get(provider_parent) {
                            Some(local_parent.clone())
                        } else if let Some(reference) = self
                            .get_tag_by_external_id(
                                library_id,
                                provider_parent,
                                Vec::new(),
                                tag_external_ids(None, provider_parent),
                                requesting_user,
                            )
                            .await?
                        {
                            provider_to_local.insert(provider_parent.clone(), reference.id.clone());
                            Some(reference.id)
                        } else if pending
                            .iter()
                            .any(|candidate| candidate.id == *provider_parent)
                        {
                            index += 1;
                            continue;
                        } else {
                            None
                        }
                    } else {
                        None
                    };
                    let reference = self
                        .resolve_or_create_refreshed_tag(library_id, tag, parent, requesting_user)
                        .await?;
                    provider_to_local.insert(tag.id.clone(), reference.id.clone());
                    push_unique_tag(&mut resolved, reference);
                    pending.remove(index);
                    progressed = true;
                }

                // Provider cycles cannot be represented locally. Break the cycle
                // at one node, then resolve its descendants normally.
                if !progressed {
                    let tag = pending.remove(0);
                    let reference = self
                        .resolve_or_create_refreshed_tag(library_id, tag, None, requesting_user)
                        .await?;
                    provider_to_local.insert(tag.id.clone(), reference.id.clone());
                    push_unique_tag(&mut resolved, reference);
                }
            }
        }

        if let Some(tags) = &relations.tags {
            for tag in tags {
                if let Some(local_id) = provider_to_local.get(&tag.id) {
                    push_unique_tag(
                        &mut resolved,
                        MediaItemReference {
                            id: local_id.clone(),
                            conf: tag.conf,
                        },
                    );
                } else if let Some(mut reference) = self
                    .get_tag_by_external_id(
                        library_id,
                        &tag.id,
                        Vec::new(),
                        tag_external_ids(None, &tag.id),
                        requesting_user,
                    )
                    .await?
                {
                    reference.conf = tag.conf;
                    push_unique_tag(&mut resolved, reference);
                }
            }
        }

        Ok(Some(resolved))
    }

    async fn resolve_or_create_refreshed_tag(
        &self,
        library_id: &str,
        tag: &crate::domain::tag::Tag,
        parent: Option<String>,
        requesting_user: &ConnectedUser,
    ) -> RsResult<MediaItemReference> {
        let store = self.store.get_library_store(library_id)?;
        let mut names = vec![tag.name.clone()];
        if let Some(alts) = &tag.alt {
            names.extend(alts.clone());
        }
        for name in names {
            if let Some(existing) = store
                .get_tags(TagQuery::new_with_name(&name))
                .await?
                .into_iter()
                .find(|candidate| candidate.parent == parent)
            {
                return Ok(MediaItemReference {
                    id: existing.id,
                    conf: Some(80),
                });
            }
        }

        let created = self
            .add_tag(
                library_id,
                TagForAdd {
                    name: tag.name.clone(),
                    parent,
                    kind: tag.kind.clone(),
                    alt: tag.alt.clone(),
                    thumb: tag.thumb.clone(),
                    params: tag.params.clone(),
                    generated: tag.generated,
                    otherids: tag_external_ids(tag.otherids.clone(), &tag.id),
                },
                requesting_user,
            )
            .await?;
        Ok(MediaItemReference {
            id: created.id,
            conf: None,
        })
    }
}

fn push_unique_tag(tags: &mut Vec<MediaItemReference>, incoming: MediaItemReference) {
    if let Some(existing) = tags.iter_mut().find(|tag| tag.id == incoming.id) {
        *existing = incoming;
    } else {
        tags.push(incoming);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tag_source_id_is_added_to_external_ids() {
        let ids = tag_external_ids(None, "provider:tag-1").unwrap();
        assert_eq!(ids.as_slice(), &["provider:tag-1".to_string()]);
    }

    #[test]
    fn duplicate_resolved_tag_uses_explicit_confidence() {
        let mut tags = vec![MediaItemReference {
            id: "local".into(),
            conf: None,
        }];
        push_unique_tag(
            &mut tags,
            MediaItemReference {
                id: "local".into(),
                conf: Some(73),
            },
        );
        assert_eq!(tags[0].conf, Some(73));

        push_unique_tag(
            &mut tags,
            MediaItemReference {
                id: "local".into(),
                conf: None,
            },
        );
        assert_eq!(tags[0].conf, None);
    }
}

//! Import plugin people using a cheap ID lookup before requesting full metadata.
use std::future::Future;

use rs_plugin_common_interfaces::{
    domain::rs_ids::RsIds,
    lookup::{RsLookupMetadataResult, RsLookupPerson, RsLookupQuery},
};

use super::{store::sql::library::SqliteLibraryStore, users::ConnectedUser, ModelController};
use crate::{
    domain::{
        library::LibraryRole,
        people::{PeopleMessage, Person, PersonWithAction},
        ElementAction,
    },
    error::RsResult,
    tools::log::{log_warn, LogServiceType},
};

#[derive(Clone, Copy)]
pub enum PeopleEntity {
    Movie,
    Serie,
}

impl PeopleEntity {
    pub(crate) fn tables(self) -> (&'static str, &'static str, &'static str) {
        match self {
            Self::Movie => ("movies", "movie_people_mapping", "movie_ref"),
            Self::Serie => ("series", "serie_people_mapping", "serie_ref"),
        }
    }
}

/// The lookup is injectable so the two-pass flow can be tested without a live plugin.
pub(crate) async fn resolve_refresh_person<F, Fut>(
    store: &SqliteLibraryStore,
    summary: Person,
    lookup: F,
) -> RsResult<Option<(Person, Option<ElementAction>)>>
where
    F: FnOnce(RsIds) -> Fut,
    Fut: Future<Output = RsResult<Option<Person>>>,
{
    let summary_kind = summary.kind.clone();
    let ids: RsIds = summary.into();
    if ids.as_all_external_ids().is_empty() {
        return Ok(None);
    }
    if let Some(existing) = store.get_person_by_external_id(ids.clone()).await? {
        return Ok(Some((existing, None)));
    }
    let Some(mut details) = lookup(ids.clone()).await? else {
        return Ok(None);
    };
    let returned_ids: RsIds = details.clone().into();
    // Never import an unrelated name-search result as this credit's person.
    if !ids.has_common_id(&returned_ids) {
        return Ok(None);
    }
    let mut combined = ids;
    combined.merge(&returned_ids);
    combined.apply_to(&mut details);
    details.kind = details.kind.or(summary_kind);
    // Recheck and create/update in one transaction, including concurrent refreshes.
    Ok(Some(store.persist_refresh_person(details).await?))
}

impl ModelController {
    /// Select metadata only when a plugin returns one of the requested external IDs.
    pub(crate) async fn lookup_person_metadata(
        &self,
        library_id: &str,
        ids: RsIds,
        user: &ConnectedUser,
    ) -> RsResult<Option<Person>> {
        user.check_library_role(library_id, LibraryRole::Read)?;
        if ids.as_all_external_ids().is_empty() {
            return Ok(None);
        }
        let mut groups = self
            .exec_lookup_metadata_grouped(
                RsLookupQuery::Person(RsLookupPerson {
                    name: None,
                    ids: Some(ids.clone()),
                    page_key: None,
                }),
                Some(library_id.to_string()),
                user,
                None,
                None,
            )
            .await?;
        super::entity_search::merge_result_ids(&mut groups);
        Ok(groups
            .into_iter()
            .flat_map(|(_, _, results)| results.results)
            .find_map(|result| match result.metadata {
                RsLookupMetadataResult::Person(person)
                    if ids.has_common_id(&person.clone().into()) =>
                {
                    Some(person)
                }
                _ => None,
            }))
    }

    pub async fn get_entity_people(
        &self,
        library_id: &str,
        entity: PeopleEntity,
        id: &str,
        user: &ConnectedUser,
    ) -> RsResult<Vec<Person>> {
        user.check_library_role(library_id, LibraryRole::Read)?;
        let store = self.store.get_library_store(library_id)?;
        store
            .get_entity_people(entity, id)
            .await
            .map_err(Into::into)
    }

    pub(crate) async fn refresh_entity_people(
        &self,
        library_id: &str,
        entity: PeopleEntity,
        id: &str,
        people: Vec<Person>,
        user: &ConnectedUser,
    ) -> RsResult<bool> {
        user.check_library_role(library_id, LibraryRole::Write)?;
        let store = self.store.get_library_store(library_id)?;
        let mut changed = false;
        for summary in people {
            let label = summary.id.clone();
            let resolved = resolve_refresh_person(&store, summary, |ids| async move {
                self.lookup_person_metadata(library_id, ids, user).await
            })
            .await;
            match resolved {
                Ok(Some((person, action))) => {
                    if let Some(action) = action {
                        self.send_people(PeopleMessage {
                            library: library_id.to_string(),
                            people: vec![PersonWithAction {
                                person: person.clone(),
                                action,
                            }],
                        });
                    }
                    changed |= store.add_entity_person(entity, id, &person.id).await?;
                }
                result => {
                    log_warn(
                        LogServiceType::Plugin,
                        format!(
                            "Skipping person {label} during refresh of {library_id}/{id}: {}",
                            match result {
                                Err(error) => error.to_string(),
                                _ => "no matching full person metadata".to_string(),
                            }
                        ),
                    );
                }
            }
        }
        Ok(changed)
    }
}

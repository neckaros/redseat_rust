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
        people::{PeopleMessage, Person, PersonType, PersonWithAction, PersonWithRoles},
        ElementAction,
    },
    error::RsResult,
    tools::log::{log_warn, LogServiceType},
};

#[derive(Clone, Copy)]
pub enum PeopleEntity {
    Movie,
    Serie,
    Book,
}

impl PeopleEntity {
    pub(crate) fn tables(self) -> (&'static str, &'static str, &'static str) {
        match self {
            Self::Book => ("books", "book_people_mapping", "book_ref"),
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
    let ids: RsIds = summary.clone().into();
    if ids.as_all_external_ids().is_empty() {
        return Ok(None);
    }
    if let Some(existing) = store.persist_existing_refresh_person(summary).await? {
        return Ok(Some(existing));
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
    pub(crate) async fn title_credit_snapshots(
        &self,
        library_id: &str,
        entity: PeopleEntity,
        ids: Vec<String>,
    ) -> RsResult<std::collections::HashMap<String, rs_plugin_common_interfaces::domain::Relations>>
    {
        Ok(self
            .store
            .get_library_store(library_id)?
            .get_people_relations_batch(entity, ids)
            .await?)
    }


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
    ) -> RsResult<Vec<PersonWithRoles>> {
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
        roles: Option<std::collections::HashMap<String, Vec<PersonType>>>,
        characters: Option<std::collections::HashMap<String, Vec<String>>>,
        ranks: Option<std::collections::HashMap<String, u32>>,
        user: &ConnectedUser,
    ) -> RsResult<bool> {
        user.check_library_role(library_id, LibraryRole::Write)?;
        let store = self.store.get_library_store(library_id)?;
        let mut changed = false;
        let mut pending: std::collections::HashMap<String, Option<Vec<PersonType>>> =
            std::collections::HashMap::new();
        let mut names: std::collections::HashMap<String, Vec<String>> =
            std::collections::HashMap::new();
        let mut resolved_ranks: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
        for summary in people {
            let label = summary.id.clone();
            let credit_roles = roles.as_ref().and_then(|roles| roles.get(&label)).cloned();
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
                    if let Some(values) = characters.as_ref().and_then(|map| map.get(&label)) {
                        let entry = names.entry(person.id.clone()).or_default();
                        for value in values {
                            if !entry.contains(value) {
                                entry.push(value.clone());
                            }
                        }
                    }
                    if let Some(rank) = ranks.as_ref().and_then(|map| map.get(&label)) {
                        let entry = resolved_ranks.entry(person.id.clone()).or_insert(*rank);
                        *entry = (*entry).min(*rank);
                    }
                    let entry = pending.entry(person.id).or_insert(None);
                    if let Some(values) = credit_roles {
                        let combined = entry.get_or_insert_with(Vec::new);
                        for role in values {
                            if !combined.contains(&role) {
                                combined.push(role);
                            }
                        }
                    }
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
        for (person_id, roles) in pending {
            changed |= store
                .upsert_entity_person_ranked_credit(
                    entity,
                    id,
                    &person_id,
                    roles,
                    names.remove(&person_id),
                    resolved_ranks.remove(&person_id),
                )
                .await?;
        }
        Ok(changed)
    }
}

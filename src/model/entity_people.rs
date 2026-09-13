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
        people::{PeopleMessage, Person, PersonWithAction, PersonWithRoles},
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

/// Combine duplicate provider credits after resolving them to one local person.
pub(crate) fn merge_credit(
    pending: &mut std::collections::HashMap<String, PersonWithRoles>,
    credit: PersonWithRoles,
) {
    use std::collections::hash_map::Entry;
    match pending.entry(credit.person.id.clone()) {
        Entry::Vacant(entry) => { entry.insert(credit); }
        Entry::Occupied(mut entry) => {
            let existing = entry.get_mut();
            merge_credit_values(&mut existing.roles, credit.roles);
            merge_credit_values(&mut existing.characters, credit.characters);
            existing.rank = match (existing.rank, credit.rank) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, b) => a.or(b),
            };
        }
    }
}

fn merge_credit_values<T: PartialEq>(existing: &mut Option<Vec<T>>, incoming: Option<Vec<T>>) {
    if let Some(incoming) = incoming {
        let values = existing.get_or_insert_default();
        for value in incoming {
            if !values.contains(&value) {
                values.push(value);
            }
        }
    }
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
        credits: Vec<PersonWithRoles>,
        user: &ConnectedUser,
    ) -> RsResult<bool> {
        user.check_library_role(library_id, LibraryRole::Write)?;
        let store = self.store.get_library_store(library_id)?;
        let mut changed = false;
        let mut pending = std::collections::HashMap::new();
        for mut credit in credits {
            let label = credit.person.id.clone();
            let resolved = resolve_refresh_person(&store, credit.person, |ids| async move {
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
                    credit.person = person;
                    merge_credit(&mut pending, credit);
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
        for (person_id, credit) in pending {
            changed |= store
                .upsert_entity_person_ranked_credit(
                    entity,
                    id,
                    &person_id,
                    credit.roles,
                    credit.characters,
                    credit.rank,
                )
                .await?;
        }
        Ok(changed)
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::people::PersonType;

    #[test]
    fn credit_merge_combines_context_without_replacing_person_profile() {
        let person = Person { id: "local".into(), name: "Saved name".into(), ..Default::default() };
        let mut pending = std::collections::HashMap::new();
        merge_credit(&mut pending, PersonWithRoles {
            person: person.clone(),
            roles: Some(vec![PersonType::Actor]),
            characters: Some(vec!["A".into()]),
            rank: Some(4),
            conf: None,
        });
        merge_credit(&mut pending, PersonWithRoles {
            person: Person { name: "Other name".into(), ..person.clone() },
            roles: Some(vec![PersonType::Actor, PersonType::Director]),
            characters: Some(vec!["A".into(), "B".into()]),
            rank: Some(0),
            conf: None,
        });
        merge_credit(&mut pending, person.clone().into());
        assert_eq!(pending.len(), 1);
        let credit = &pending["local"];
        assert_eq!(credit.person, person);
        assert_eq!(credit.roles, Some(vec![PersonType::Actor, PersonType::Director]));
        assert_eq!(credit.characters, Some(vec!["A".into(), "B".into()]));
        assert_eq!(credit.rank, Some(0));
    }

    #[test]
    fn credit_merge_preserves_explicit_empty_and_unknown_fields() {
        let person = Person { id: "local".into(), ..Default::default() };
        let mut pending = std::collections::HashMap::new();
        merge_credit(&mut pending, person.clone().into());
        merge_credit(&mut pending, PersonWithRoles {
            person,
            roles: Some(vec![]),
            ..Default::default()
        });
        assert_eq!(pending["local"].roles, Some(vec![]));
        assert_eq!(pending["local"].characters, None);
        assert_eq!(pending["local"].rank, None);
    }
}

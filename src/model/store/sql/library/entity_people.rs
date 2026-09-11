use rs_plugin_common_interfaces::domain::rs_ids::RsIds;
use rusqlite::{params, params_from_iter};

use super::{Result, SqliteLibraryStore};
use crate::{
    domain::{people::Person, ElementAction},
    model::{entity_people::PeopleEntity, store::to_pipe_separated_optional},
};

impl SqliteLibraryStore {
    pub(super) fn find_person_by_ids(
        conn: &rusqlite::Connection,
        ids: &RsIds,
    ) -> rusqlite::Result<Option<Person>> {
        let mut conditions = Vec::new();
        let mut values = Vec::new();
        for key in ["imdb", "slug", "tmdb", "trakt"] {
            if let Some(value) = ids.get(key) {
                conditions.push(format!("{key} = ?"));
                values.push(value.to_string());
            }
        }
        for external_id in ids.as_all_external_ids() {
            conditions.push(
                "EXISTS (SELECT 1 FROM json_each(people.otherids) WHERE value = ?)".to_string(),
            );
            values.push(external_id);
        }
        if conditions.is_empty() {
            return Ok(None);
        }
        let mut query = conn.prepare(&format!(
            "SELECT {} FROM people WHERE {} ORDER BY id LIMIT 2",
            Self::PEOPLE_FIELDS,
            conditions.join(" OR "),
        ))?;
        let mut matches = query.query_map(params_from_iter(values), Self::row_to_person)?;
        let first = matches.next().transpose()?;
        if matches.next().transpose()?.is_some() {
            return Err(rusqlite::Error::InvalidParameterName(
                "Conflicting person IDs match multiple library people".to_string(),
            ));
        }
        Ok(first)
    }

    /// Adds missing IDs only to existing profiles; rechecks inside the write transaction.
    pub(crate) async fn persist_refresh_person(
        &self,
        mut incoming: Person,
    ) -> Result<(Person, Option<ElementAction>)> {
        Ok(self.connection.call(move |conn| {
            let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            let ids: RsIds = incoming.clone().into();
            let (id, action) = if let Some(existing) = Self::find_person_by_ids(&tx, &ids)? {
                let mut merged = existing.clone();
                let mut merged_ids: RsIds = existing.clone().into();
                merged_ids.merge(&ids);
                merged_ids.apply_to(&mut merged);
                // Dedicated IDs already set by the user take precedence, including
                // when older otherids entries contain a different value.
                merged.imdb = existing.imdb.clone().or(merged.imdb);
                merged.tmdb = existing.tmdb.or(merged.tmdb);
                merged.trakt = existing.trakt.or(merged.trakt);
                merged.slug = existing.slug.clone().or(merged.slug);
                let changed = merged.imdb != existing.imdb || merged.tmdb != existing.tmdb
                    || merged.trakt != existing.trakt || merged.slug != existing.slug
                    || merged.otherids != existing.otherids;
                if changed {
                    tx.execute("UPDATE people SET imdb = ?, tmdb = ?, trakt = ?, slug = ?, otherids = ? WHERE id = ?",
                        params![merged.imdb, merged.tmdb, merged.trakt, merged.slug, merged.otherids, existing.id])?;
                }
                (existing.id, changed.then_some(ElementAction::Updated))
            } else {
                incoming.id = nanoid::nanoid!();
                let socials = incoming.socials.as_ref().map(serde_json::to_string).transpose()
                    .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
                tx.execute("INSERT INTO people (id, name, socials, type, alt, portrait, params, birthday, generated, imdb, slug, tmdb, trakt, death, gender, country, bio, otherids)
                    VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)", params![
                    incoming.id, incoming.name, socials, incoming.kind,
                    to_pipe_separated_optional(incoming.alt), incoming.portrait, incoming.params,
                    incoming.birthday, incoming.generated, incoming.imdb, incoming.slug, incoming.tmdb,
                    incoming.trakt, incoming.death, incoming.gender, incoming.country, incoming.bio, incoming.otherids,
                ])?;
                (incoming.id, Some(ElementAction::Added))
            };
            let person = tx.query_row(&format!("SELECT {} FROM people WHERE id = ?", Self::PEOPLE_FIELDS), [&id], Self::row_to_person)?;
            tx.commit()?;
            Ok((person, action))
        }).await?)
    }

    pub(crate) async fn add_entity_person(
        &self,
        entity: PeopleEntity,
        id: &str,
        person_id: &str,
    ) -> Result<bool> {
        let id = id.to_string();
        let person_id = person_id.to_string();
        Ok(self
            .connection
            .call(move |conn| {
                let (parent, mapping, reference) = entity.tables();
                // Existence checks also protect installations with foreign_keys disabled.
                Ok(conn.execute(
                    &format!(
                        "INSERT INTO {mapping} ({reference}, people_ref)
                SELECT ?, ? WHERE EXISTS (SELECT 1 FROM {parent} WHERE id = ?)
                AND EXISTS (SELECT 1 FROM people WHERE id = ?)
                ON CONFLICT ({reference}, people_ref) DO NOTHING"
                    ),
                    params![id, person_id, id, person_id],
                )? > 0)
            })
            .await?)
    }

    pub(crate) async fn get_entity_people(
        &self,
        entity: PeopleEntity,
        id: &str,
    ) -> Result<Vec<Person>> {
        let id = id.to_string();
        Ok(self
            .connection
            .call(move |conn| {
                let (_, mapping, reference) = entity.tables();
                let mut query = conn.prepare(&format!(
                    "SELECT {} FROM people
                WHERE id IN (SELECT people_ref FROM {mapping} WHERE {reference} = ?)
                ORDER BY name, id",
                    Self::PEOPLE_FIELDS
                ))?;
                let people = query
                    .query_map([id], Self::row_to_person)?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(people)
            })
            .await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        error::RsResult,
        model::{
            entity_people::resolve_refresh_person,
            people::{PeopleQuery, PersonForAdd, PersonForInsert},
        },
    };

    async fn store() -> SqliteLibraryStore {
        SqliteLibraryStore::new(tokio_rusqlite::Connection::open_in_memory().await.unwrap())
            .await
            .unwrap()
    }

    fn summary() -> Person {
        Person {
            id: "tmdb:42".into(),
            name: "Plugin Name".into(),
            tmdb: Some(42),
            ..Default::default()
        }
    }

    async fn existing(store: &SqliteLibraryStore, tmdb: Option<u64>) {
        store
            .add_person(PersonForInsert {
                id: "local-person".into(),
                person: PersonForAdd {
                    name: "User's Name".into(),
                    bio: Some("User's biography".into()),
                    imdb: Some("nm0042".into()),
                    tmdb,
                    ..Default::default()
                },
            })
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn refresh_people_uses_credit_type_only_when_details_omit_it() {
        use crate::domain::people::PersonType;
        let store = store().await;
        let credit = Person {
            kind: Some(PersonType::Director),
            ..summary()
        };
        let (created, _) =
            resolve_refresh_person(&store, credit.clone(), |_| async { Ok(Some(summary())) })
                .await
                .unwrap()
                .unwrap();
        assert_eq!(created.kind, Some(PersonType::Director));
        let (reused, _) = resolve_refresh_person(
            &store,
            Person {
                kind: Some(PersonType::Actor),
                ..credit
            },
            |_| async {
                panic!("Existing person must retain their type without a lookup");
                #[allow(unreachable_code)]
                Ok(None)
            },
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(reused.id, created.id);
        assert_eq!(reused.kind, Some(PersonType::Director));
    }

    #[tokio::test]
    async fn refresh_people_type_json_and_database_compatibility() {
        use crate::domain::people::PersonType;
        use crate::model::people::PersonForUpdate;

        let store = store().await;
        for (index, (wire, expected)) in [
            ("Actor", PersonType::Actor),
            ("Director", PersonType::Director),
            ("Author", PersonType::Author),
            ("Family", PersonType::Family),
            ("Friends", PersonType::Friends),
            ("Singer", PersonType::Singer),
            ("custom name", PersonType::Custom("custom name".into())),
            ("Acting", PersonType::Custom("Acting".into())),
        ]
        .into_iter()
        .enumerate()
        {
            let id = format!("person-{index}");
            let add: PersonForAdd = serde_json::from_value(serde_json::json!({
                "name": "Person", "type": wire
            }))
            .unwrap();
            assert_eq!(add.kind, Some(expected.clone()));
            assert_eq!(serde_json::to_value(&add).unwrap()["type"], wire);
            store
                .add_person(PersonForInsert {
                    id: id.clone(),
                    person: add,
                })
                .await
                .unwrap();
            let person = store.get_person(&id).await.unwrap().unwrap();
            assert_eq!(person.kind, Some(expected));
            assert_eq!(serde_json::to_value(person).unwrap()["type"], wire);

            let update: PersonForUpdate = serde_json::from_value(serde_json::json!({
                "type": "custom updated"
            }))
            .unwrap();
            store.update_person(&id, update).await.unwrap();
            assert_eq!(
                store.get_person(&id).await.unwrap().unwrap().kind,
                Some(PersonType::Custom("custom updated".into()))
            );
        }
        // Existing TEXT values remain readable without rewriting or alias mapping.
        store
            .connection
            .call(|conn| {
                conn.execute(
                    "UPDATE people SET type = 'acteur' WHERE id = 'person-0'",
                    [],
                )?;
                Ok(())
            })
            .await
            .unwrap();
        assert_eq!(
            store.get_person("person-0").await.unwrap().unwrap().kind,
            Some(PersonType::Custom("acteur".into()))
        );
    }

    #[tokio::test]
    async fn refresh_people_merge_preserves_movie_and_show_links() {
        let store = store().await;
        existing(&store, None).await;
        store
            .add_person(PersonForInsert {
                id: "target".into(),
                person: PersonForAdd {
                    name: "Target".into(),
                    ..Default::default()
                },
            })
            .await
            .unwrap();
        store
            .connection
            .call(|conn| {
                conn.execute(
                    "INSERT INTO movies (id, name) VALUES ('movie', 'Movie')",
                    [],
                )?;
                conn.execute("INSERT INTO series (id, name) VALUES ('serie', 'Show')", [])?;
                Ok(())
            })
            .await
            .unwrap();
        for (entity, id) in [
            (PeopleEntity::Movie, "movie"),
            (PeopleEntity::Serie, "serie"),
        ] {
            store
                .add_entity_person(entity, id, "local-person")
                .await
                .unwrap();
        }
        store
            .add_entity_person(PeopleEntity::Movie, "movie", "target")
            .await
            .unwrap();
        store
            .transfer_faces_between_people("local-person", "target")
            .await
            .unwrap();
        store
            .transfer_faces_between_people("target", "target")
            .await
            .unwrap();
        for (entity, id) in [
            (PeopleEntity::Movie, "movie"),
            (PeopleEntity::Serie, "serie"),
        ] {
            let people = store.get_entity_people(entity, id).await.unwrap();
            assert_eq!(people.len(), 1);
            assert_eq!(people[0].id, "target");
        }
    }

    #[tokio::test]
    async fn refresh_people_known_id_skips_plugin() {
        let store = store().await;
        existing(&store, Some(42)).await;
        let (person, action) = resolve_refresh_person(&store, summary(), |_| async {
            panic!("Known person must not require a plugin lookup");
            #[allow(unreachable_code)]
            Ok(None)
        })
        .await
        .unwrap()
        .unwrap();
        assert_eq!(person.id, "local-person");
        assert!(action.is_none());
    }

    #[tokio::test]
    async fn refresh_people_enriched_imdb_reuses_profile_and_remembers_tmdb() {
        let store = store().await;
        existing(&store, None).await;
        let (person, action) = resolve_refresh_person(&store, summary(), |ids| async move {
            assert_eq!(ids.tmdb(), Some(42));
            Ok(Some(Person {
                imdb: Some("nm0042".into()),
                bio: Some("Provider biography".into()),
                ..summary()
            }))
        })
        .await
        .unwrap()
        .unwrap();
        assert_eq!(person.id, "local-person");
        assert_eq!(person.name, "User's Name");
        assert_eq!(person.bio.as_deref(), Some("User's biography"));
        assert_eq!(person.tmdb, Some(42));
        assert!(matches!(action, Some(ElementAction::Updated)));
        assert_eq!(
            store
                .get_people(PeopleQuery::default())
                .await
                .unwrap()
                .len(),
            1
        );
        resolve_refresh_person(&store, summary(), |_| async {
            panic!("TMDB ID should now match locally");
            #[allow(unreachable_code)]
            Ok(None)
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn refresh_people_creates_full_profile_once_even_concurrently() {
        let store = store().await;
        let barrier = tokio::sync::Barrier::new(2);
        let run = || {
            resolve_refresh_person(&store, summary(), |_| async {
                barrier.wait().await;
                Ok(Some(Person {
                    imdb: Some("nm0042".into()),
                    bio: Some("Full profile".into()),
                    kind: Some(crate::domain::people::PersonType::Actor),
                    ..summary()
                }))
            })
        };
        let (first, second) = tokio::join!(run(), run());
        let first = first.unwrap().unwrap();
        let second = second.unwrap().unwrap();
        assert_eq!(first.0.id, second.0.id);
        assert_ne!(first.0.id, "tmdb:42");
        assert_eq!(first.0.bio.as_deref(), Some("Full profile"));
        assert_eq!(first.0.kind, Some(crate::domain::people::PersonType::Actor));
        assert_eq!(
            [first.1, second.1]
                .iter()
                .filter(|action| matches!(action, Some(ElementAction::Added)))
                .count(),
            1
        );
        assert_eq!(
            store
                .get_people(PeopleQuery::default())
                .await
                .unwrap()
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn refresh_people_failed_or_unrelated_details_do_not_create() {
        let store = store().await;
        let failure: RsResult<Option<(Person, Option<ElementAction>)>> =
            resolve_refresh_person(&store, summary(), |_| async {
                Err(
                    crate::model::error::Error::ServiceError("Plugin unavailable".into(), None)
                        .into(),
                )
            })
            .await;
        assert!(failure.is_err());
        assert!(
            resolve_refresh_person(&store, summary(), |_| async { Ok(None) })
                .await
                .unwrap()
                .is_none()
        );
        assert!(resolve_refresh_person(&store, summary(), |_| async {
            Ok(Some(Person {
                id: "tmdb:99".into(),
                tmdb: Some(99),
                ..Default::default()
            }))
        })
        .await
        .unwrap()
        .is_none());
        assert!(store
            .get_people(PeopleQuery::default())
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn refresh_people_matches_exact_otherids_without_sentinel_or_wildcard_matches() {
        let store = store().await;
        store
            .add_person(PersonForInsert {
                id: "other-person".into(),
                person: PersonForAdd {
                    name: "Other Person".into(),
                    tmdb: Some(0),
                    otherids: Some(vec!["imdb:nm0042".into(), "custom:a_b".into()].into()),
                    ..Default::default()
                },
            })
            .await
            .unwrap();
        assert!(store
            .get_person_by_external_id(RsIds::default())
            .await
            .unwrap()
            .is_none());
        assert!(store
            .get_person_by_external_id("custom:a%b".to_string().try_into().unwrap())
            .await
            .unwrap()
            .is_none());
        assert_eq!(
            store
                .get_person_by_external_id("imdb:nm0042".to_string().try_into().unwrap())
                .await
                .unwrap()
                .unwrap()
                .id,
            "other-person"
        );
        let (person, _) = resolve_refresh_person(&store, summary(), |_| async {
            Ok(Some(Person {
                imdb: Some("nm0042".into()),
                ..summary()
            }))
        })
        .await
        .unwrap()
        .unwrap();
        assert_eq!(person.id, "other-person");
    }

    #[tokio::test]
    async fn refresh_people_conflicting_matches_do_not_merge_arbitrarily() {
        let store = store().await;
        existing(&store, None).await;
        store
            .add_person(PersonForInsert {
                id: "second".into(),
                person: PersonForAdd {
                    name: "Second".into(),
                    tmdb: Some(42),
                    ..Default::default()
                },
            })
            .await
            .unwrap();
        let incoming = Person {
            imdb: Some("nm0042".into()),
            ..summary()
        };
        assert!(store.persist_refresh_person(incoming).await.is_err());
        assert_eq!(
            store
                .get_people(PeopleQuery::default())
                .await
                .unwrap()
                .len(),
            2
        );
    }

    #[tokio::test]
    async fn refresh_people_relationships_are_idempotent_and_cleaned_up() {
        let store = store().await;
        existing(&store, Some(42)).await;
        store.connection.call(|conn| {
            conn.execute_batch("INSERT INTO movies (id, name) VALUES ('movie', 'Movie'); INSERT INTO series (id, name) VALUES ('serie', 'Show');")?;
            Ok(())
        }).await.unwrap();
        for (kind, id) in [
            (PeopleEntity::Movie, "movie"),
            (PeopleEntity::Serie, "serie"),
        ] {
            assert!(store
                .add_entity_person(kind, id, "local-person")
                .await
                .unwrap());
            assert!(!store
                .add_entity_person(kind, id, "local-person")
                .await
                .unwrap());
            let people = store.get_entity_people(kind, id).await.unwrap();
            assert_eq!(people.len(), 1);
            assert_eq!(people[0].id, "local-person");
        }
        assert!(!store
            .add_entity_person(PeopleEntity::Movie, "missing", "local-person")
            .await
            .unwrap());
        store.remove_movie("movie".into()).await.unwrap();
        assert!(store
            .get_entity_people(PeopleEntity::Movie, "movie")
            .await
            .unwrap()
            .is_empty());
        assert!(store.get_person("local-person").await.unwrap().is_some());
        store.remove_person("local-person".into()).await.unwrap();
        assert!(store
            .get_entity_people(PeopleEntity::Serie, "serie")
            .await
            .unwrap()
            .is_empty());
    }
    #[tokio::test]
    async fn refresh_people_keeps_summary_ids_omitted_from_full_details() {
        let store = store().await;
        let credit = Person {
            otherids: Some(vec!["custom:credit42".into()].into()),
            ..summary()
        };
        let (person, _) = resolve_refresh_person(&store, credit, |_| async {
            Ok(Some(Person {
                imdb: Some("nm0042".into()),
                ..summary()
            }))
        })
        .await
        .unwrap()
        .unwrap();
        assert_eq!(
            person.otherids.unwrap().get("custom").as_deref(),
            Some("credit42")
        );
    }

    #[tokio::test]
    async fn refresh_people_migration_preserves_existing_data() {
        let store = store().await;
        existing(&store, Some(42)).await;
        store
            .connection
            .call(|conn| {
                conn.execute_batch(
                    "DROP TABLE movie_people_mapping;
                DROP TABLE serie_people_mapping;
                DROP TRIGGER delete_movie_people;
                DROP TRIGGER delete_serie_people;
                DROP TRIGGER delete_person_entities;
                PRAGMA user_version = 54;
                INSERT INTO series (id, name) VALUES ('show', 'Existing Show');",
                )?;
                Ok(())
            })
            .await
            .unwrap();
        assert_eq!(store.migrate().await.unwrap(), 55);
        assert_eq!(
            store
                .get_person("local-person")
                .await
                .unwrap()
                .unwrap()
                .name,
            "User's Name"
        );
        assert!(store
            .add_entity_person(PeopleEntity::Serie, "show", "local-person")
            .await
            .unwrap());
        store.remove_serie("show".into()).await.unwrap();
        assert!(store
            .get_entity_people(PeopleEntity::Serie, "show")
            .await
            .unwrap()
            .is_empty());
        assert!(store.get_person("local-person").await.unwrap().is_some());
    }
}

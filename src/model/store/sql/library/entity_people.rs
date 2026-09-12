use rs_plugin_common_interfaces::domain::rs_ids::RsIds;
use rusqlite::{params, params_from_iter};

use super::{Result, SqliteLibraryStore};
use crate::{
    domain::{
        people::{Person, PersonType, PersonWithRoles},
        ElementAction,
    },
    model::{entity_people::PeopleEntity, store::to_pipe_separated_optional},
};

impl SqliteLibraryStore {
    /// One mapping query for the complete page/event, including empty snapshots.
    pub(crate) async fn get_people_relations_batch(
        &self,
        entity: PeopleEntity,
        ids: Vec<String>,
    ) -> Result<std::collections::HashMap<String, rs_plugin_common_interfaces::domain::Relations>>
    {
        if ids.is_empty() {
            return Ok(std::collections::HashMap::new());
        }
        Ok(self
            .connection
            .call(move |conn| Ok(Self::load_people_relations(conn, entity, &ids)?))
            .await?)
    }

    pub(super) fn load_people_relations(
        conn: &rusqlite::Connection,
        entity: PeopleEntity,
        ids: &[String],
    ) -> rusqlite::Result<
        std::collections::HashMap<String, rs_plugin_common_interfaces::domain::Relations>,
    > {
        use rs_plugin_common_interfaces::domain::{media::MediaItemReference, Relations};
        use std::collections::HashMap;
        let mut snapshots: HashMap<String, Relations> = ids
            .iter()
            .map(|id| {
                (
                    id.clone(),
                    Relations {
                        people: Some(Vec::new()),
                        people_roles: Some(HashMap::new()),
                        people_characters: Some(HashMap::new()),
                        ..Default::default()
                    },
                )
            })
            .collect();
        let mut query = conn.prepare(&Self::people_relations_sql(entity))?;
        let mut rows = query.query([serde_json::json!(ids)])?;
        while let Some(row) = rows.next()? {
            let id: String = row.get(0)?;
            let person: String = row.get(1)?;
            let roles: Option<String> = row.get(2)?;
            let characters: Option<String> = row.get(3)?;
            let parse_error = |error| {
                rusqlite::Error::FromSqlConversionFailure(
                    0,
                    rusqlite::types::Type::Text,
                    Box::new(error),
                )
            };
            let roles = roles
                .map(|raw| serde_json::from_str::<Vec<PersonType>>(&raw))
                .transpose()
                .map_err(parse_error)?
                .unwrap_or_default();
            let characters = characters
                .map(|raw| serde_json::from_str::<Vec<String>>(&raw))
                .transpose()
                .map_err(parse_error)?
                .unwrap_or_default();
            if let Some(relations) = snapshots.get_mut(&id) {
                relations
                    .people
                    .get_or_insert_default()
                    .push(MediaItemReference {
                        id: person.clone(),
                        conf: row.get(4)?,
                    });
                relations
                    .people_roles
                    .get_or_insert_default()
                    .insert(person.clone(), roles);
                relations
                    .people_characters
                    .get_or_insert_default()
                    .insert(person, characters);
            }
        }
        Ok(snapshots)
    }

    fn people_relations_sql(entity: PeopleEntity) -> String {
        let (_, mapping, reference) = entity.tables();
        let confidence = if matches!(entity, PeopleEntity::Book) {
            "m.confidence"
        } else {
            "NULL"
        };
        format!(
            "SELECT m.{reference}, m.people_ref, m.roles, m.characters, {confidence}
            FROM json_each(?) requested JOIN {mapping} m ON m.{reference} = requested.value
            ORDER BY m.{reference}, m.people_ref"
        )
    }

    pub(super) fn add_people_filter(
        query: &mut super::super::RsQueryBuilder,
        entity: PeopleEntity,
        person: Option<String>,
        role: Option<PersonType>,
    ) {
        use super::super::SqlWhereType;
        let (_, mapping, reference) = entity.tables();
        let (sql, value): (String, Box<dyn rusqlite::ToSql>) = match (person, role) {
            (Some(person), Some(role)) => (
                format!("id IN (SELECT m.{reference} FROM json_each(?) wanted
                    JOIN {mapping} m ON m.people_ref = wanted.key
                    WHERE EXISTS (SELECT 1 FROM json_each(m.roles) role WHERE role.value = wanted.value))"),
                Box::new(serde_json::json!({ (person): role })),
            ),
            (Some(person), None) => (format!("id IN (SELECT {reference} FROM {mapping} WHERE people_ref = ?)"), Box::new(person)),
            (None, Some(role)) => (format!("id IN (SELECT {reference} FROM {mapping}
                WHERE EXISTS (SELECT 1 FROM json_each(roles) WHERE value = ?))"), Box::new(role)),
            (None, None) => return,
        };
        query.add_where(SqlWhereType::Custom(sql, value));
    }

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

    /// Enrich a known profile without ever creating one from a credit summary.
    pub(crate) async fn persist_existing_refresh_person(
        &self,
        incoming: Person,
    ) -> Result<Option<(Person, Option<ElementAction>)>> {
        self.persist_refresh_person_inner(incoming, false).await
    }

    pub(crate) async fn persist_refresh_person(
        &self,
        incoming: Person,
    ) -> Result<(Person, Option<ElementAction>)> {
        self.persist_refresh_person_inner(incoming, true)
            .await?
            .ok_or_else(|| rusqlite::Error::QueryReturnedNoRows.into())
    }

    /// Match and enrich/create atomically; existing profiles receive missing IDs only.
    async fn persist_refresh_person_inner(
        &self,
        mut incoming: Person,
        allow_create: bool,
    ) -> Result<Option<(Person, Option<ElementAction>)>> {
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
            } else if !allow_create {
                return Ok(None);
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
            Ok(Some((person, action)))
        }).await?)
    }

    pub(crate) async fn add_entity_person(
        &self,
        entity: PeopleEntity,
        id: &str,
        person_id: &str,
    ) -> Result<bool> {
        self.upsert_entity_person_roles(entity, id, person_id, None)
            .await
    }

    pub(crate) async fn upsert_entity_person_roles(
        &self,
        entity: PeopleEntity,
        id: &str,
        person_id: &str,
        roles: Option<Vec<PersonType>>,
    ) -> Result<bool> {
        self.upsert_entity_person_credit(entity, id, person_id, roles, None)
            .await
    }

    pub(crate) async fn upsert_entity_person_credit(
        &self,
        entity: PeopleEntity,
        id: &str,
        person_id: &str,
        roles: Option<Vec<PersonType>>,
        characters: Option<Vec<String>>,
    ) -> Result<bool> {
        let characters = characters.map(|mut values| {
            values.sort();
            values.dedup();
            serde_json::json!(values)
        });
        let roles = roles.map(|mut roles| {
            roles.sort_by(|a, b| a.as_str().cmp(b.as_str()));
            roles.dedup();
            serde_json::json!(roles)
        });
        let id = id.to_string();
        let person_id = person_id.to_string();
        Ok(self
            .connection
            .call(move |conn| {
                let (parent, mapping, reference) = entity.tables();
                // Existence checks also protect installations with foreign_keys disabled.
                Ok(conn.execute(
                    &format!(
                        "INSERT INTO {mapping} ({reference}, people_ref, roles, characters)
                SELECT ?, ?, ?, ? WHERE EXISTS (SELECT 1 FROM {parent} WHERE id = ?)
                AND EXISTS (SELECT 1 FROM people WHERE id = ?)
                ON CONFLICT ({reference}, people_ref) DO UPDATE SET
                    roles = coalesce(excluded.roles, {mapping}.roles),
                    characters = coalesce(excluded.characters, {mapping}.characters)
                WHERE (excluded.roles IS NOT NULL AND {mapping}.roles IS NOT excluded.roles)
                   OR (excluded.characters IS NOT NULL AND {mapping}.characters IS NOT excluded.characters)"
                    ),
                    params![id, person_id, roles, characters, id, person_id],
                )? > 0)
            })
            .await?)
    }

    pub(crate) async fn get_entity_people(
        &self,
        entity: PeopleEntity,
        id: &str,
    ) -> Result<Vec<PersonWithRoles>> {
        let id = id.to_string();
        Ok(self
            .connection
            .call(move |conn| {
                let (_, mapping, reference) = entity.tables();
                let mut query = conn.prepare(&format!(
                    "SELECT {}, (SELECT roles FROM {mapping} WHERE {reference} = ? AND people_ref = people.id), (SELECT characters FROM {mapping} WHERE {reference} = ? AND people_ref = people.id) FROM people
                WHERE id IN (SELECT people_ref FROM {mapping} WHERE {reference} = ?)
                ORDER BY name, id",
                    Self::PEOPLE_FIELDS
                ))?;
                let people = query
                    .query_map([&id, &id, &id], |row| {
                        let raw: Option<String> = row.get(Self::PEOPLE_FIELDS.split(',').count())?;
                        let roles = raw.map(|raw| serde_json::from_str(&raw)).transpose()
                            .map_err(|e| rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e)))?;
                        let raw: Option<String> = row.get(Self::PEOPLE_FIELDS.split(',').count() + 1)?;
                        let characters = raw.map(|raw| serde_json::from_str(&raw)).transpose()
                            .map_err(|e| rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e)))?;
                        Ok(PersonWithRoles { person: Self::row_to_person(row)?, roles, characters })
                    })?
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
    async fn credit_snapshots_clear_removed_links_and_advance_sync_for_all_titles() {
        use crate::model::{books::BookQuery, movies::MovieQuery, series::SerieQuery};
        let store = store().await;
        existing(&store, Some(42)).await;
        store
            .connection
            .call(|conn| {
                conn.execute_batch(
                    "INSERT INTO movies(id,name) VALUES ('title','Movie'), ('empty','Empty');
                INSERT INTO series(id,name) VALUES ('title','Show'), ('empty','Empty');
                INSERT INTO books(id,name) VALUES ('title','Book'), ('empty','Empty');",
                )?;
                Ok(())
            })
            .await
            .unwrap();
        for entity in [PeopleEntity::Movie, PeopleEntity::Serie, PeopleEntity::Book] {
            store
                .connection
                .call(move |conn| {
                    let mut query = conn.prepare(&format!(
                        "EXPLAIN QUERY PLAN {}",
                        SqliteLibraryStore::people_relations_sql(entity)
                    ))?;
                    let plan = query
                        .query_map([serde_json::json!(["title", "empty"])], |row| {
                            row.get::<_, String>(3)
                        })?
                        .collect::<rusqlite::Result<Vec<_>>>()?;
                    let (_, _, reference) = entity.tables();
                    assert!(
                        plan.iter().any(|step| step.contains("SEARCH m USING")
                            && step.contains(&format!("{reference}=?"))),
                        "{plan:?}"
                    );
                    assert!(
                        !plan.iter().any(|step| step.starts_with("SCAN m")),
                        "{plan:?}"
                    );
                    Ok(())
                })
                .await
                .unwrap();

            store
                .upsert_entity_person_credit(
                    entity,
                    "title",
                    "local-person",
                    Some(vec![PersonType::Actor, PersonType::Custom("Guest".into())]),
                    Some(vec!["Narrator".into()]),
                )
                .await
                .unwrap();
            let snapshot = store
                .get_people_relations_batch(entity, vec!["title".into(), "empty".into()])
                .await
                .unwrap();
            let title = serde_json::to_value(&snapshot["title"]).unwrap();
            assert_eq!(title["people"][0]["id"], "local-person");
            assert_eq!(
                title["peopleRoles"]["local-person"],
                serde_json::json!(["Actor", "Guest"])
            );
            assert_eq!(
                title["peopleCharacters"]["local-person"],
                serde_json::json!(["Narrator"])
            );
            assert_eq!(
                serde_json::to_value(&snapshot["empty"]).unwrap(),
                serde_json::json!({"people":[], "peopleRoles":{}, "peopleCharacters":{}})
            );
            // Live event title objects keep flat metadata and embed the same snapshot.
            let event = match entity {
                PeopleEntity::Movie => serde_json::to_value(crate::domain::movie::MoviesMessage {
                    library: "library".into(),
                    movies: vec![crate::domain::movie::MovieWithAction {
                        action: ElementAction::Updated,
                        movie: rs_plugin_common_interfaces::domain::ItemWithRelations {
                            item: crate::domain::movie::Movie {
                                id: "title".into(),
                                ..Default::default()
                            },
                            relations: Some(snapshot["title"].clone()),
                        },
                    }],
                })
                .unwrap(),
                PeopleEntity::Serie => serde_json::to_value(crate::domain::serie::SeriesMessage {
                    library: "library".into(),
                    series: vec![crate::domain::serie::SerieWithAction {
                        action: ElementAction::Updated,
                        serie: rs_plugin_common_interfaces::domain::ItemWithRelations {
                            item: crate::domain::serie::Serie {
                                id: "title".into(),
                                ..Default::default()
                            },
                            relations: Some(snapshot["title"].clone()),
                        },
                    }],
                })
                .unwrap(),
                PeopleEntity::Book => serde_json::to_value(crate::domain::book::BooksMessage {
                    library: "library".into(),
                    books: vec![crate::domain::book::BookWithAction {
                        action: ElementAction::Updated,
                        book: rs_plugin_common_interfaces::domain::ItemWithRelations {
                            item: crate::domain::book::Book {
                                id: "title".into(),
                                ..Default::default()
                            },
                            relations: Some(snapshot["title"].clone()),
                        },
                    }],
                })
                .unwrap(),
            };
            let (plural, singular) = match entity {
                PeopleEntity::Movie => ("movies", "movie"),
                PeopleEntity::Serie => ("series", "serie"),
                PeopleEntity::Book => ("books", "book"),
            };
            assert_eq!(event[plural][0][singular]["id"], "title");
            assert_eq!(event[plural][0][singular]["relations"], title);
            assert!(event[plural][0][singular].get("item").is_none());
            // Force a future cursor: even changes in the same millisecond must advance it.
            let cursor = 4_000_000_000_000_i64;
            store
                .connection
                .call(move |conn| {
                    let (table, _, _) = entity.tables();
                    conn.execute(
                        &format!("UPDATE {table} SET modified = ? WHERE id = 'title'"),
                        [cursor],
                    )?;
                    Ok(())
                })
                .await
                .unwrap();
            store
                .upsert_entity_person_credit(
                    entity,
                    "title",
                    "local-person",
                    Some(vec![]),
                    Some(vec![]),
                )
                .await
                .unwrap();
            let cleared = store
                .get_people_relations_batch(entity, vec!["title".into()])
                .await
                .unwrap();
            assert_eq!(
                cleared["title"].people_roles.as_ref().unwrap()["local-person"],
                vec![]
            );
            assert_eq!(
                cleared["title"].people_characters.as_ref().unwrap()["local-person"],
                Vec::<String>::new()
            );
            let count = match entity {
                PeopleEntity::Movie => store
                    .get_movies(MovieQuery {
                        after: Some(cursor),
                        ..Default::default()
                    })
                    .await
                    .unwrap()
                    .len(),
                PeopleEntity::Serie => store
                    .get_series(SerieQuery {
                        after: Some(cursor),
                        ..Default::default()
                    })
                    .await
                    .unwrap()
                    .len(),
                PeopleEntity::Book => store
                    .get_books(BookQuery {
                        after: Some(cursor),
                        ..Default::default()
                    })
                    .await
                    .unwrap()
                    .len(),
            };
            assert_eq!(count, 1);
            store
                .connection
                .call(move |conn| {
                    let (table, mapping, reference) = entity.tables();
                    let before: i64 = conn.query_row(
                        &format!("SELECT modified FROM {table} WHERE id = 'title'"),
                        [],
                        |r| r.get(0),
                    )?;
                    conn.execute(
                        &format!("DELETE FROM {mapping} WHERE {reference} = 'title'"),
                        [],
                    )?;
                    let after: i64 = conn.query_row(
                        &format!("SELECT modified FROM {table} WHERE id = 'title'"),
                        [],
                        |r| r.get(0),
                    )?;
                    assert!(after > before);
                    Ok(())
                })
                .await
                .unwrap();
            let removed = store
                .get_people_relations_batch(entity, vec!["title".into()])
                .await
                .unwrap();
            assert_eq!(
                serde_json::to_value(&removed["title"]).unwrap(),
                serde_json::json!({"people":[], "peopleRoles":{}, "peopleCharacters":{}})
            );
        }
    }

    #[tokio::test]
    async fn relationship_roles_filter_matches_the_same_person_and_uses_index() {
        use crate::model::{books::BookQuery, movies::MovieQuery, series::SerieQuery};
        let store = store().await;
        existing(&store, Some(42)).await;
        store
            .add_person(PersonForInsert {
                id: "other".into(),
                person: PersonForAdd {
                    name: "Other".into(),
                    ..Default::default()
                },
            })
            .await
            .unwrap();
        store
            .connection
            .call(|conn| {
                conn.execute_batch(
                    "INSERT INTO movies(id,name) VALUES ('movie','Movie');
                INSERT INTO series(id,name) VALUES ('serie','Show');
                INSERT INTO books(id,name) VALUES ('book','Book');",
                )?;
                Ok(())
            })
            .await
            .unwrap();
        for (entity, id) in [
            (PeopleEntity::Movie, "movie"),
            (PeopleEntity::Serie, "serie"),
            (PeopleEntity::Book, "book"),
        ] {
            store
                .upsert_entity_person_roles(
                    entity,
                    id,
                    "local-person",
                    Some(vec![PersonType::Actor]),
                )
                .await
                .unwrap();
            store
                .upsert_entity_person_roles(entity, id, "other", Some(vec![PersonType::Director]))
                .await
                .unwrap();
        }
        assert!(store
            .get_movies(MovieQuery {
                person: Some("local-person".into()),
                role: Some(PersonType::Director),
                ..Default::default()
            })
            .await
            .unwrap()
            .is_empty());
        assert_eq!(
            store
                .get_movies(MovieQuery {
                    person: Some("other".into()),
                    role: Some(PersonType::Director),
                    ..Default::default()
                })
                .await
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            store
                .get_series(SerieQuery {
                    person: Some("other".into()),
                    role: Some(PersonType::Director),
                    ..Default::default()
                })
                .await
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            store
                .get_books(BookQuery {
                    person: Some("other".into()),
                    role: Some(PersonType::Director),
                    ..Default::default()
                })
                .await
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            store
                .get_movies(MovieQuery {
                    role: Some(PersonType::Actor),
                    ..Default::default()
                })
                .await
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            store
                .get_movies(MovieQuery {
                    person: Some("local-person".into()),
                    ..Default::default()
                })
                .await
                .unwrap()
                .len(),
            1
        );
        store
            .connection
            .call(|conn| {
                let mut query = super::super::super::RsQueryBuilder::new();
                SqliteLibraryStore::add_people_filter(
                    &mut query,
                    PeopleEntity::Movie,
                    Some("other".into()),
                    Some(PersonType::Director),
                );
                let mut statement = conn.prepare(&format!(
                    "EXPLAIN QUERY PLAN SELECT id FROM movies {}",
                    query.format()
                ))?;
                let plan = statement
                    .query_map(query.values(), |row| row.get::<_, String>(3))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                assert!(
                    plan.iter().any(|line| line.contains("movie_people_person")),
                    "{plan:?}"
                );
                Ok(())
            })
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn relationship_roles_character_names_survive_partial_updates_and_merges() {
        let store = store().await;
        existing(&store, Some(42)).await;
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
                conn.execute("INSERT INTO movies(id,name) VALUES ('movie','Movie')", [])?;
                Ok(())
            })
            .await
            .unwrap();
        store
            .upsert_entity_person_credit(
                PeopleEntity::Movie,
                "movie",
                "local-person",
                Some(vec![PersonType::Actor]),
                Some(vec!["B".into(), "A".into(), "A".into()]),
            )
            .await
            .unwrap();
        store
            .upsert_entity_person_roles(
                PeopleEntity::Movie,
                "movie",
                "local-person",
                Some(vec![PersonType::Director]),
            )
            .await
            .unwrap();
        assert_eq!(
            store
                .get_entity_people(PeopleEntity::Movie, "movie")
                .await
                .unwrap()[0]
                .characters,
            Some(vec!["A".into(), "B".into()])
        );
        store
            .upsert_entity_person_credit(
                PeopleEntity::Movie,
                "movie",
                "target",
                None,
                Some(vec!["C".into()]),
            )
            .await
            .unwrap();
        store
            .transfer_faces_between_people("local-person", "target")
            .await
            .unwrap();
        let credit = store
            .get_entity_people(PeopleEntity::Movie, "movie")
            .await
            .unwrap()
            .remove(0);
        assert_eq!(
            credit.characters,
            Some(vec!["A".into(), "B".into(), "C".into()])
        );
        assert_eq!(credit.roles, Some(vec![PersonType::Director]));
        store
            .upsert_entity_person_credit(PeopleEntity::Movie, "movie", "target", None, Some(vec![]))
            .await
            .unwrap();
        let credit = store
            .get_entity_people(PeopleEntity::Movie, "movie")
            .await
            .unwrap()
            .remove(0);
        assert_eq!(credit.characters, Some(vec![]));
        assert_eq!(credit.roles, Some(vec![PersonType::Director]));
    }

    #[tokio::test]
    async fn relationship_roles_are_nullable_idempotent_and_independent_of_profile() {
        use crate::domain::people::PersonType;
        let store = store().await;
        existing(&store, Some(42)).await;
        store
            .connection
            .call(|conn| {
                conn.execute_batch(
                    "INSERT INTO movies (id,name) VALUES ('movie','Movie');
                INSERT INTO series (id,name) VALUES ('serie','Show');
                INSERT INTO books (id,name) VALUES ('book','Book');",
                )?;
                Ok(())
            })
            .await
            .unwrap();
        for (entity, id) in [
            (PeopleEntity::Movie, "movie"),
            (PeopleEntity::Serie, "serie"),
            (PeopleEntity::Book, "book"),
        ] {
            store
                .add_entity_person(entity, id, "local-person")
                .await
                .unwrap();
            assert!(store.get_entity_people(entity, id).await.unwrap()[0]
                .roles
                .is_none());
            let roles = vec![
                PersonType::Director,
                PersonType::Actor,
                PersonType::Actor,
                PersonType::Custom("custom name".into()),
            ];
            assert!(store
                .upsert_entity_person_roles(entity, id, "local-person", Some(roles.clone()))
                .await
                .unwrap());
            assert!(!store
                .upsert_entity_person_roles(entity, id, "local-person", Some(roles))
                .await
                .unwrap());
            assert!(!store
                .add_entity_person(entity, id, "local-person")
                .await
                .unwrap());
            let credit = store.get_entity_people(entity, id).await.unwrap().remove(0);
            assert_eq!(
                credit.roles.unwrap(),
                vec![
                    PersonType::Actor,
                    PersonType::Director,
                    PersonType::Custom("custom name".into())
                ]
            );
            assert_eq!(credit.person.kind, None);
            let (table, mapping, reference) = entity.tables();
            store
                .connection
                .call(move |conn| {
                    let count: i64 = conn.query_row(
                        &format!(
                            "SELECT count(*) FROM {mapping} WHERE people_ref = 'local-person'
                    AND EXISTS (SELECT 1 FROM json_each(roles) WHERE value = 'Director')"
                        ),
                        [],
                        |r| r.get(0),
                    )?;
                    assert_eq!(count, 1);
                    conn.execute(
                        &format!("UPDATE {table} SET modified=4000000000000 WHERE id=?"),
                        [id],
                    )?;
                    conn.execute(
                        &format!("UPDATE {mapping} SET roles=? WHERE {reference}=?"),
                        params![serde_json::json!(["Author"]), id],
                    )?;
                    let modified: i64 = conn.query_row(
                        &format!("SELECT modified FROM {table} WHERE id=?"),
                        [id],
                        |r| r.get(0),
                    )?;
                    assert!(modified > 4000000000000);
                    Ok(())
                })
                .await
                .unwrap();
            assert!(store
                .upsert_entity_person_roles(entity, id, "local-person", Some(vec![]))
                .await
                .unwrap());
            assert_eq!(
                store.get_entity_people(entity, id).await.unwrap()[0].roles,
                Some(vec![])
            );
        }
    }

    #[tokio::test]
    async fn relationship_roles_merge_unions_source_and_target() {
        let store = store().await;
        existing(&store, Some(42)).await;
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
        store.connection.call(|conn| {
            conn.execute_batch("INSERT INTO movies(id,name) VALUES ('movie','Movie'); INSERT INTO books(id,name) VALUES ('book','Book');")?;
            Ok(())
        }).await.unwrap();
        for (entity, id) in [(PeopleEntity::Movie, "movie"), (PeopleEntity::Book, "book")] {
            store
                .upsert_entity_person_roles(
                    entity,
                    id,
                    "local-person",
                    Some(vec![PersonType::Director]),
                )
                .await
                .unwrap();
            store
                .upsert_entity_person_roles(entity, id, "target", Some(vec![PersonType::Actor]))
                .await
                .unwrap();
        }
        store
            .transfer_faces_between_people("local-person", "target")
            .await
            .unwrap();
        for (entity, id) in [(PeopleEntity::Movie, "movie"), (PeopleEntity::Book, "book")] {
            let people = store.get_entity_people(entity, id).await.unwrap();
            assert_eq!(people.len(), 1);
            assert_eq!(
                people[0].roles,
                Some(vec![PersonType::Actor, PersonType::Director])
            );
        }
    }

    #[tokio::test]
    async fn refresh_people_fast_path_persists_summary_ids_without_replacing_profile() {
        let store = store().await;
        existing(&store, None).await;
        let credit = Person {
            imdb: Some("nm0042".into()),
            kind: Some(crate::domain::people::PersonType::Actor),
            otherids: Some(vec!["custom:42".into()].into()),
            ..summary()
        };
        let (person, action) = resolve_refresh_person(&store, credit.clone(), |_| async {
            panic!("A matching summary must not fetch full details");
            #[allow(unreachable_code)]
            Ok(None)
        })
        .await
        .unwrap()
        .unwrap();
        assert_eq!(person.id, "local-person");
        assert_eq!(person.tmdb, Some(42));
        assert_eq!(person.name, "User's Name");
        assert_eq!(person.bio.as_deref(), Some("User's biography"));
        assert_eq!(person.kind, None);
        assert_eq!(
            person.otherids.unwrap().get("custom").as_deref(),
            Some("42")
        );
        assert!(matches!(action, Some(ElementAction::Updated)));

        for summary in [credit, summary()] {
            let (person, action) = resolve_refresh_person(&store, summary, |_| async {
                panic!("Saved IDs must resolve later credits without fetching details");
                #[allow(unreachable_code)]
                Ok(None)
            })
            .await
            .unwrap()
            .unwrap();
            assert_eq!(person.id, "local-person");
            assert!(action.is_none());
        }
    }

    #[tokio::test]
    async fn refresh_people_relationship_sync_strictly_advances_after_metadata() {
        let store = store().await;
        existing(&store, Some(42)).await;
        store
            .connection
            .call(|conn| {
                conn.execute_batch(
                    "
                INSERT INTO movies (id, name) VALUES ('movie', 'Movie');
                INSERT INTO series (id, name) VALUES ('serie', 'Show');
            ",
                )?;
                Ok(())
            })
            .await
            .unwrap();
        for (entity, id) in [
            (PeopleEntity::Movie, "movie"),
            (PeopleEntity::Serie, "serie"),
        ] {
            let (table, _, _) = entity.tables();
            let cursor = store
                .connection
                .call(move |conn| {
                    // A future timestamp makes a same-clock-tick regression deterministic.
                    conn.execute(
                        &format!("UPDATE {table} SET modified = 4000000000000 WHERE id = ?"),
                        [id],
                    )?;
                    conn.execute(
                        &format!("UPDATE {table} SET name = 'Refreshed' WHERE id = ?"),
                        [id],
                    )?;
                    Ok(conn.query_row(
                        &format!("SELECT modified FROM {table} WHERE id = ?"),
                        [id],
                        |row| row.get::<_, i64>(0),
                    )?)
                })
                .await
                .unwrap();
            assert!(cursor > 4000000000000);
            assert!(store
                .add_entity_person(entity, id, "local-person")
                .await
                .unwrap());
            let linked = store
                .connection
                .call(move |conn| {
                    Ok(conn.query_row(
                        &format!("SELECT modified FROM {table} WHERE id = ? AND modified > ?"),
                        params![id, cursor],
                        |row| row.get::<_, i64>(0),
                    )?)
                })
                .await
                .unwrap();
            assert_eq!(linked, cursor + 1);
            assert!(!store
                .add_entity_person(entity, id, "local-person")
                .await
                .unwrap());
            let (_, mapping, reference) = entity.tables();
            store
                .connection
                .call(move |conn| {
                    let unchanged: i64 = conn.query_row(
                        &format!("SELECT modified FROM {table} WHERE id = ?"),
                        [id],
                        |row| row.get(0),
                    )?;
                    assert_eq!(unchanged, linked);
                    conn.execute(
                        &format!("DELETE FROM {mapping} WHERE {reference} = ?"),
                        [id],
                    )?;
                    let deleted: i64 = conn.query_row(
                        &format!("SELECT modified FROM {table} WHERE id = ? AND modified > ?"),
                        params![id, linked],
                        |row| row.get(0),
                    )?;
                    assert_eq!(deleted, linked + 1);
                    Ok(())
                })
                .await
                .unwrap();
        }
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
            assert_eq!(people[0].person.id, "target");
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
            assert_eq!(people[0].person.id, "local-person");
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
                DROP TRIGGER modified_book_people_mapping_roles;
                ALTER TABLE book_people_mapping DROP COLUMN roles;
                ALTER TABLE book_people_mapping DROP COLUMN characters;
                PRAGMA user_version = 54;
                INSERT INTO series (id, name) VALUES ('show', 'Existing Show');",
                )?;
                Ok(())
            })
            .await
            .unwrap();
        assert_eq!(store.migrate().await.unwrap(), 57);
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

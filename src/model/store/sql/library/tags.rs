use nanoid::nanoid;
use rs_plugin_common_interfaces::domain::other_ids::OtherIds;
use rusqlite::{params, params_from_iter, OptionalExtension, Row};

use super::{Result, SqliteLibraryStore};
use crate::model::Error;
use crate::{
    domain::tag::{Tag, TagForUpdate},
    model::{
        store::{
            from_pipe_separated_optional,
            sql::{pagination_clause, OrderBuilder, QueryBuilder, QueryWhereType, SqlOrder},
            to_pipe_separated_optional,
        },
        tags::{TagForAdd, TagForInsert, TagQuery},
    },
    plugins::sources::error::SourcesError,
    tools::array_tools::replace_add_remove_from_array,
};

impl SqliteLibraryStore {
    fn row_to_tag(row: &Row) -> rusqlite::Result<Tag> {
        Ok(Tag {
            id: row.get(0)?,
            name: row.get(1)?,
            parent: row.get(2)?,
            kind: row.get(3)?,
            alt: from_pipe_separated_optional(row.get(4)?),
            thumb: row.get(5)?,
            params: row.get(6)?,
            modified: row.get(7)?,
            added: row.get(8)?,
            generated: row.get(9)?,
            path: row.get(10)?,
            otherids: row.get(11)?,
        })
    }

    pub async fn get_tag_descendants(&self, prefix: &str) -> Result<Vec<Tag>> {
        let prefix = prefix.to_string();
        let tags = self.connection.call(move |conn| {
            let mut query = conn.prepare("SELECT id, name, parent, type, alt, thumb, params, modified, added, generated, path, otherids FROM tags WHERE substr(path, 1, ?1) = ?2")?;
            let rows = query.query_map(params![prefix.chars().count() as i64, prefix], Self::row_to_tag)?;
            Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
        }).await?;
        Ok(tags)
    }

    pub async fn get_tags(&self, query: TagQuery) -> Result<Vec<Tag>> {
        let pagination = pagination_clause(query.limit, query.offset)?;
        let row = self.connection.call( move |conn| { 
            let mut where_query = QueryBuilder::new();
            
            if let Some(q) = &query.parent {
                where_query.add_where(QueryWhereType::Equal("parent", q));
            }
            if let Some(q) = &query.path {
                where_query.add_where(QueryWhereType::Equal("path", q));
            }
            if let Some(q) = &query.after {
                where_query.add_where(QueryWhereType::After("modified", q));
            }

            if query.after.is_some() || query.limit.is_some() || query.offset.is_some() {
                where_query.add_oder(OrderBuilder::new("modified".to_string(), SqlOrder::ASC));
                where_query.add_oder(OrderBuilder::new("id".to_string(), SqlOrder::ASC));
            }


            if let Some(q) = &query.name {
                where_query.add_where(QueryWhereType::EqualWithAlt("name", "alt", "|", q));
                //println!("q '{}'", q)
            }
            //println!("sql: {}", where_query.format());

            let mut query = conn.prepare(&format!("SELECT id, name, parent, type, alt, thumb, params, modified, added, generated, path, otherids  FROM tags {}{}{}", where_query.format(), where_query.format_order(), pagination))?;
            
            let rows = query.query_map(
            where_query.values(), Self::row_to_tag,
            )?;
            let backups:Vec<Tag> = rows.collect::<std::result::Result<Vec<Tag>, rusqlite::Error>>()?; 
            //println!("results: {:?}", backups);
            Ok(backups)
        }).await?;
        Ok(row)
    }
    pub async fn get_tag(&self, credential_id: &str) -> Result<Option<Tag>> {
        let credential_id = credential_id.to_string();
        let row = self.connection.call( move |conn| {
            let mut query = conn.prepare("SELECT id, name, parent, type, alt, thumb, params, modified, added, generated, path, otherids FROM tags WHERE id = ?")?;
            let row = query.query_row(
            [credential_id],Self::row_to_tag).optional()?;
            Ok(row)
        }).await?;
        Ok(row)
    }

    /// Returns the first tag whose stored `otherids` JSON overlaps any entry in the provided list.
    /// Each stored entry looks like `"platform:value"` inside a JSON array, so we match with LIKE `%"entry"%`.
    pub async fn get_tag_by_otherids(&self, otherids: OtherIds) -> Result<Option<Tag>> {
        if otherids.0.is_empty() {
            return Ok(None);
        }
        let row = self.connection.call(move |conn| {
            let conditions = otherids.0.iter()
                .map(|_| "otherids LIKE ?")
                .collect::<Vec<_>>()
                .join(" OR ");
            let sql = format!(
                "SELECT id, name, parent, type, alt, thumb, params, modified, added, generated, path, otherids FROM tags WHERE {}",
                conditions
            );
            let like_params: Vec<String> = otherids.0.iter()
                .map(|entry| format!("%\"{}\"%" , entry))
                .collect();
            let mut query = conn.prepare(&sql)?;
            let row = query.query_row(params_from_iter(like_params.iter()), Self::row_to_tag).optional()?;
            Ok(row)
        }).await?;
        Ok(row)
    }

    pub async fn update_tag(&self, tag_id: &str, update: TagForUpdate) -> Result<()> {
        let id = tag_id.to_string();
        let existing_tag = self.get_tag(&tag_id).await?.ok_or_else(|| {
            SourcesError::UnableToFindTag(
                "store".to_string(),
                tag_id.to_string(),
                "update_serie".to_string(),
            )
        })?;
        self.connection.call( move |conn| { 
            let tx = conn.transaction()?;
            let mut where_query = QueryBuilder::new();
            where_query.add_update(&update.name, "name");
            // An empty parent ID is an explicit move to root. Omission keeps
            // the current parent, preserving the existing PATCH contract.
            if update.parent.as_deref() == Some("") {
                where_query.add_nullify("parent");
            } else {
                where_query.add_update(&update.parent, "parent");
            }
            where_query.add_update(&update.kind, "type");

            let alts = replace_add_remove_from_array(existing_tag.alt.clone(), update.alt, update.add_alts, update.remove_alts);
            let v = to_pipe_separated_optional(alts);
            where_query.add_update(&v, "alt");

            where_query.add_update(&update.thumb, "thumb");
            where_query.add_update(&update.params, "params");

            let otherids = replace_add_remove_from_array(
                existing_tag.otherids.clone().map(|o| o.into_vec()),
                update.otherids.map(|o| o.into_vec()),
                update.add_otherids,
                update.remove_otherids,
            );
            let otherids: Option<OtherIds> = otherids.map(OtherIds::from);
            where_query.add_update(&otherids, "otherids");

            let generated = Some(update.generated.unwrap_or_default());
            where_query.add_update(&generated, "generated");
            


            where_query.add_where(QueryWhereType::Equal("id", &id));
            

            let update_sql = format!("UPDATE Tags SET {} {}", where_query.format_update(), where_query.format());
            tx.execute(&update_sql, where_query.values())?;

            if let Some(migrate_to) = &update.migrate_to {
                tx.execute("UPDATE or IGNORE media_tag_mapping SET tag_ref = ? where tag_ref = ?", params![&migrate_to, id])?;
                println!("MIGRATE: UPDATE media_tag_mapping SET tag_ref = {} where tag_ref = {}", id, migrate_to);
                tx.execute("DELETE FROM media_tag_mapping  WHERE tag_ref = ?", [&id])?;
                tx.execute("UPDATE OR IGNORE channel_tag_mapping SET tag_ref = ? WHERE tag_ref = ?", params![&migrate_to, id])?;
                tx.execute("DELETE FROM channel_tag_mapping WHERE tag_ref = ?", [&id])?;
            }
            
            if update.name.is_some() || update.parent.is_some() {
                let new_path = match update.parent.as_deref() {
                    Some("") => "/".to_string(),
                    Some(parent_id) => {
                        let parent = tx.query_row(
                            "SELECT id, name, parent, type, alt, thumb, params, modified, added, generated, path, otherids FROM tags WHERE id = ?",
                            [parent_id], Self::row_to_tag,
                        )?;
                        if parent.id == existing_tag.id || parent.path.starts_with(&existing_tag.childs_path()) {
                            return Err(tokio_rusqlite::Error::Other(std::io::Error::new(std::io::ErrorKind::InvalidInput, "A tag cannot be moved beneath itself or its descendants").into()));
                        }
                        parent.childs_path()
                    }
                    None => existing_tag.path.clone(),
                };
                let new_name = update.name.as_deref().unwrap_or(&existing_tag.name);
                let old_prefix = existing_tag.childs_path();
                let new_prefix = format!("{}{}/", new_path, new_name);
                let prefix_length = old_prefix.chars().count() as i64;
                tx.execute("UPDATE tags SET path = ? WHERE id = ?", params![new_path, &existing_tag.id])?;
                // Match the literal prefix, including wildcard characters in
                // names, and only replace the leading ancestor path.
                tx.execute(
                    "UPDATE tags SET path = ?1 || substr(path, ?2 + 1) WHERE substr(path, 1, ?2) = ?3",
                    params![new_prefix, prefix_length, old_prefix],
                )?;
            }

            tx.commit()?;
            Ok(())
        }).await?;

        Ok(())
    }

    pub async fn add_tag(&self, tag: TagForInsert) -> Result<()> {
        self.connection.call( move |conn| { 
            let new_path = if let Some(parent) = &tag.parent {
                let mut query_parent = conn.prepare("SELECT id, name, parent, type, alt, thumb, params, modified, added, generated, path, otherids FROM tags WHERE id = ?")?;
                let parent = query_parent.query_row(&[&parent],Self::row_to_tag)?;
                parent.childs_path()
            } else {
                String::from("/")
            };
            
            conn.execute("INSERT INTO tags (id, name, parent, type, alt, thumb, params, generated, path, otherids)
            VALUES (?, ?, ? ,?, ?, ?, ?, ?, ?, ?)", params![
                tag.id,
                tag.name,
                tag.parent,
                tag.kind,
                to_pipe_separated_optional(tag.alt),
                tag.thumb,
                tag.params,
                tag.generated,
                new_path,
                tag.otherids
            ])?;
            
            Ok(())
        }).await?;
        Ok(())
    }

    pub async fn get_or_create_path(
        &self,
        mut path: Vec<&str>,
        template: TagForUpdate,
    ) -> Result<Tag> {
        let path_string = path.join("/");
        let tag_by_path = self
            .get_tags(TagQuery::new_with_path(path_string))
            .await?
            .into_iter()
            .nth(0);
        if let Some(tag) = tag_by_path {
            return Ok(tag);
        }
        let mut parent: Option<String> = None;

        let last_element = path
            .pop()
            .ok_or(Error::ServiceError("Empty path".into(), None))?;

        for element in path {
            let previous_parent = parent.clone();

            let tag_by_name_and_parent = self
                .get_tags(TagQuery::new_with_name_and_parent(
                    element,
                    previous_parent.clone(),
                ))
                .await?
                .into_iter()
                .nth(0);
            parent = if let Some(parent) = tag_by_name_and_parent {
                Some(parent.id.clone())
            } else {
                let id = nanoid!();
                self.add_tag(TagForInsert {
                    id: id.clone(),
                    name: element.to_string(),
                    parent: previous_parent,
                    generated: template.generated.unwrap_or(false),
                    alt: template.alt.clone(),
                    ..Default::default()
                })
                .await?;
                Some(id)
            }
        }

        let mut all_names = template.alt.clone().unwrap_or(vec![]);
        all_names.insert(0, last_element.to_string());

        for name in all_names {
            let tag_by_name_and_parent = self
                .get_tags(TagQuery::new_with_name_and_parent(
                    &name,
                    parent.as_ref().and_then(|t| Some(t.clone())),
                ))
                .await?
                .into_iter()
                .nth(0);
            if let Some(tag) = tag_by_name_and_parent {
                return Ok(tag);
            }
        }
        let new_tag_id = nanoid!();
        self.add_tag(TagForInsert {
            id: new_tag_id.clone(),
            name: last_element.to_string(),
            parent: parent.and_then(|t| Some(t.clone())),
            generated: template.generated.unwrap_or(false),
            alt: template.alt.clone(),
            ..Default::default()
        })
        .await?;
        let result_tag = self
            .get_tag(&new_tag_id)
            .await?
            .ok_or(Error::TagNotFound(new_tag_id.clone()))?;
        Ok(result_tag)
    }

    pub async fn remove_tag(&self, tag_id: String) -> Result<()> {
        self.connection.call( move |conn| { 
            let tx = conn.transaction()?;
            
            let existing = tx.query_row("SELECT id, name, parent, type, alt, thumb, params, modified, added, generated, path, otherids FROM tags WHERE id = ?", &[&tag_id],Self::row_to_tag)?;

            tx.execute("DELETE FROM tags WHERE id = ?", &[&tag_id])?;
            tx.execute("DELETE FROM book_tag_mapping  WHERE tag_ref = ?", &[&tag_id])?;
            tx.execute("DELETE FROM media_tag_mapping  WHERE tag_ref = ?", &[&tag_id])?;
            tx.execute("DELETE FROM channel_tag_mapping WHERE tag_ref = ?", &[&tag_id])?;
            tx.execute("DELETE FROM tags WHERE path like ?", &[&format!("{}%", existing.childs_path())])?;

            tx.commit()?;
            Ok(())
        }).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn tag_root_store() -> SqliteLibraryStore {
        let store =
            SqliteLibraryStore::new(tokio_rusqlite::Connection::open_in_memory().await.unwrap())
                .await
                .unwrap();
        for (id, name, parent) in [
            ("p", "Parent", None),
            ("t", "Tag_%é", Some("p")),
            ("c", "Child", Some("t")),
            ("other", "TagXXé", Some("p")),
            ("other-child", "Other child", Some("other")),
        ] {
            store
                .add_tag(TagForInsert {
                    id: id.into(),
                    name: name.into(),
                    parent: parent.map(str::to_string),
                    ..Default::default()
                })
                .await
                .unwrap();
        }
        store
    }

    #[tokio::test]
    async fn tag_root_move_updates_descendants_and_is_retryable() {
        let store = tag_root_store().await;
        for _ in 0..2 {
            store
                .update_tag(
                    "t",
                    TagForUpdate {
                        parent: Some(String::new()),
                        ..Default::default()
                    },
                )
                .await
                .unwrap();
            let tag = store.get_tag("t").await.unwrap().unwrap();
            assert_eq!(tag.parent, None);
            assert_eq!(tag.path, "/");
            let descendants = store.get_tag_descendants(&tag.childs_path()).await.unwrap();
            assert_eq!(descendants.len(), 1);
            assert_eq!(descendants[0].id, "c");
            assert_eq!(store.get_tag("c").await.unwrap().unwrap().path, "/Tag_%é/");
            assert_eq!(
                store.get_tag("other-child").await.unwrap().unwrap().path,
                "/Parent/TagXXé/"
            );
        }
    }

    #[tokio::test]
    async fn tag_root_move_with_rename_updates_descendants_once() {
        let store = tag_root_store().await;
        store
            .update_tag(
                "t",
                TagForUpdate {
                    parent: Some(String::new()),
                    name: Some("Renamed".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(store.get_tag("c").await.unwrap().unwrap().path, "/Renamed/");
    }

    #[tokio::test]
    async fn tag_root_omission_preserves_parent_and_invalid_moves_roll_back() {
        let store = tag_root_store().await;
        store
            .update_tag(
                "t",
                TagForUpdate {
                    name: Some("Renamed".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(
            store.get_tag("t").await.unwrap().unwrap().parent.as_deref(),
            Some("p")
        );
        for parent in ["missing", "t", "c"] {
            assert!(store
                .update_tag(
                    "t",
                    TagForUpdate {
                        parent: Some(parent.into()),
                        ..Default::default()
                    }
                )
                .await
                .is_err());
            assert_eq!(
                store.get_tag("t").await.unwrap().unwrap().parent.as_deref(),
                Some("p")
            );
        }
        store
            .update_tag(
                "t",
                TagForUpdate {
                    parent: Some("other".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(
            store.get_tag("c").await.unwrap().unwrap().path,
            "/Parent/TagXXé/Renamed/"
        );
    }
}

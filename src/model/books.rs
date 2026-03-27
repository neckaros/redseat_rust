use std::io::Cursor;

use async_recursion::async_recursion;
use nanoid::nanoid;
use rs_plugin_common_interfaces::{
    ExternalImage, ImageType, domain::{ItemWithRelations, other_ids::OtherIds, rs_ids::{ApplyRsIds, RsIds}}, lookup::{RsLookupBook, RsLookupMetadataResult, RsLookupQuery},
};
use serde::{Deserialize, Serialize};
use strum_macros::EnumString;

use crate::{
    domain::{
        book::{Book, BookForUpdate, BookWithAction, BooksMessage},
        deleted::RsDeleted,
        library::LibraryRole,
        ElementAction, MediaElement,
    },
    error::RsResult,
    model::{
        people::PersonForAdd,
        tags::TagForAdd,
    },
    plugins::sources::{
        error::SourcesError, AsyncReadPinBox, FileStreamResult,
    },
    routes::sse::SseEvent,
    tools::image_tools::{convert_image_reader, ImageSize},
};

use super::{
    entity_images::EntityImageConfig,
    entity_search::merge_result_ids,
    error::{Error, Result},
    store::sql::SqlOrder,
    users::ConnectedUser,
    ModelController,
};

#[derive(
    Debug, Serialize, Deserialize, Clone, PartialEq, strum_macros::Display, EnumString, Default,
)]
#[strum(serialize_all = "camelCase")]
#[serde(rename_all = "camelCase")]
pub enum RsBookSort {
    #[default]
    Modified,
    Added,
    Name,
    Year,
    Volume,
    Chapter,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct BookQuery {
    pub after: Option<i64>,
    pub name: Option<String>,
    pub serie_ref: Option<String>,
    pub isbn13: Option<String>,
    pub openlibrary_edition_id: Option<String>,
    pub openlibrary_work_id: Option<String>,
    pub google_books_volume_id: Option<String>,
    pub asin: Option<String>,
    #[serde(default)]
    pub sort: RsBookSort,
    #[serde(default)]
    pub order: SqlOrder,
}

impl ModelController {
    pub async fn get_books(
        &self,
        library_id: &str,
        query: BookQuery,
        requesting_user: &ConnectedUser,
    ) -> RsResult<Vec<ItemWithRelations<Book>>> {
        requesting_user.check_library_role(library_id, LibraryRole::Read)?;
        let store = self.store.get_library_store(library_id)?;
        Ok(store.get_books(query).await?)
    }

    pub async fn get_book(
        &self,
        library_id: &str,
        book_id: String,
        requesting_user: &ConnectedUser,
    ) -> RsResult<ItemWithRelations<Book>> {
        requesting_user.check_library_role(library_id, LibraryRole::Read)?;
        let store = self.store.get_library_store(library_id)?;
        if RsIds::is_id(&book_id) {
            let ids: RsIds = book_id.clone().try_into().map_err(|_| {
                SourcesError::UnableToFindMovie(
                    library_id.to_string(),
                    book_id.clone(),
                    "get_book".to_string(),
                )
            })?;
            if let Some(book) = store.get_book_by_external_id(ids.clone()).await? {
                Ok(book)
            } else {
                // Try plugin lookup first
                let lookup_query = RsLookupQuery::Book(RsLookupBook {
                    name: Some(String::new()),
                    ids: Some(ids.clone()),
                    author: None,
                    page_key: None,
                });
                let plugin_results = self
                    .exec_lookup_metadata_grouped(
                        lookup_query,
                        Some(library_id.to_string()),
                        requesting_user,
                        None,
                        None,
                    )
                    .await?;
                let plugin_book = plugin_results.into_iter().flat_map(|(_, _, r)| r.results).find_map(|result| {
                    let relations = result.relations;
                    match result.metadata {
                        RsLookupMetadataResult::Book(book) => Some(ItemWithRelations {
                            item: book,
                            relations,
                        }),
                        _ => None,
                    }
                });
                plugin_book.ok_or(
                    SourcesError::UnableToFindMovie(
                        library_id.to_string(),
                        format!("{:?}", ids),
                        "get_book".to_string(),
                    )
                    .into(),
                )
            }
        } else {
            store
                .get_book(&book_id)
                .await?
                .ok_or(
                    SourcesError::UnableToFindMovie(
                        library_id.to_string(),
                        book_id,
                        "get_book".to_string(),
                    )
                    .into(),
                )
        }
    }

    pub async fn get_book_by_external_id(
        &self,
        library_id: &str,
        ids: RsIds,
        requesting_user: &ConnectedUser,
    ) -> RsResult<Option<ItemWithRelations<Book>>> {
        requesting_user.check_library_role(library_id, LibraryRole::Read)?;
        let store = self.store.get_library_store(library_id)?;
        Ok(store.get_book_by_external_id(ids).await?)
    }

    fn validate_book(new_book: &Book) -> Result<()> {
        if new_book.name.trim().is_empty() {
            return Err(Error::NotFound("book name is required".to_string()));
        }
        if new_book.chapter.is_some() && new_book.serie_ref.is_none() {
            return Err(Error::ServiceError(
                "invalid book".to_string(),
                Some("chapter requires serie_ref".to_string()),
            ));
        }
        Ok(())
    }

    pub async fn add_book(
        &self,
        library_id: &str,
        item: ItemWithRelations<Book>,
        upsert_tags: bool,
        upsert_people: bool,
        upsert_serie: bool,
        requesting_user: &ConnectedUser,
    ) -> RsResult<Book> {
        requesting_user.check_library_role(library_id, LibraryRole::Write)?;
        let mut new_book = item.item;
        let relations = item.relations;
        Self::validate_book(&new_book)?;

        // Resolve or upsert series_details into new_book.serie_ref
        if let Some(rel) = &relations {
            if let Some(series_details) = &rel.series_details {
                if let Some(serie) = series_details.first() {
                    if let Some(found) = self.get_serie_by_any_id(library_id, serie, requesting_user).await? {
                        new_book.serie_ref = Some(found.id);
                    } else if upsert_serie {
                        let created = self.add_serie(library_id, serie.clone(), requesting_user).await?;
                        new_book.serie_ref = Some(created.id);
                    }
                }
            }
        }

        let ids: RsIds = new_book.clone().into();
        new_book.apply_rs_ids(&ids);
        println!("Adding book with ids: {:?}", ids);
        println!("Adding book {:?}", new_book);
        if ids.isbn13().is_some()
            || ids.openlibrary_edition_id().is_some()
            || ids.openlibrary_work_id().is_some()
            || ids.google_books_volume_id().is_some()
            || ids.asin().is_some()
        {
            if let Some(existing) = self
                .get_book_by_external_id(library_id, ids, requesting_user)
                .await?
            {
                return Err(
                    Error::Duplicate(existing.item.id.to_owned(), MediaElement::Book(existing.item)).into(),
                );
            }
        }

        let store = self.store.get_library_store(library_id)?;
        new_book.id = nanoid!();
        store.add_book(new_book.clone()).await?;

        // Wire up raw tag references
        if let Some(rel) = &relations {
            if let Some(tags) = &rel.tags {
                for tag_ref in tags {
                    store.add_book_tag(&new_book.id, &tag_ref.id, tag_ref.conf.map(|c| c as i32)).await?;
                }
            }
            if let Some(people) = &rel.people {
                for person_ref in people {
                    store.add_book_person(&new_book.id, &person_ref.id, person_ref.conf.map(|c| c as i32)).await?;
                }
            }

            // Wire up tags_details with upsert logic
            if let Some(tags_details) = &rel.tags_details {
                for tag in tags_details {
                    let mut names = vec![tag.name.clone()];
                    if let Some(alts) = &tag.alt {
                        names.extend(alts.clone());
                    }
                    if let Some(found) = self.get_tag_by_external_id(
                        library_id,
                        &tag.id,
                        names,
                        tag.otherids.clone(),
                        requesting_user,
                    ).await? {
                        store.add_book_tag(&new_book.id, &found.id, found.conf.map(|c| c as i32)).await?;
                    } else if upsert_tags {
                        let created = self.add_tag(library_id, TagForAdd {
                            name: tag.name.clone(),
                            parent: tag.parent.clone(),
                            kind: tag.kind.clone(),
                            alt: tag.alt.clone(),
                            thumb: tag.thumb.clone(),
                            params: tag.params.clone(),
                            generated: tag.generated,
                            otherids: tag.otherids.clone(),
                        }, requesting_user).await?;
                        store.add_book_tag(&new_book.id, &created.id, None).await?;
                    }
                }
            }

            // Wire up people_details with upsert logic
            if let Some(people_details) = &rel.people_details {
                for person in people_details {
                    let person_ids: RsIds = person.clone().into();
                    let external_ids = person_ids.as_all_external_ids();
                    if let Some(_existing_person) = store.get_person(&person.id).await? {
                        store.add_book_person(&new_book.id, &person.id, None).await?;
                    } else if let Some(found) = store.get_person_by_external_id(person_ids).await? {
                        store.add_book_person(&new_book.id, &found.id, Some(80)).await?;
                    } else if upsert_people {
                        let mut otherids = person.otherids.clone().unwrap_or_default();
                        for ext_id in external_ids {
                            if let Some((key, value)) = ext_id.split_once(':') {
                                otherids.add(key, value);
                            }
                        }
                        let created = self.add_pesron(library_id, PersonForAdd {
                            name: person.name.clone(),
                            socials: person.socials.clone(),
                            kind: person.kind.clone(),
                            alt: person.alt.clone(),
                            portrait: person.portrait.clone(),
                            params: person.params.clone(),
                            birthday: person.birthday,
                            generated: person.generated,
                            imdb: person.imdb.clone(),
                            slug: person.slug.clone(),
                            tmdb: person.tmdb,
                            trakt: person.trakt,
                            death: person.death,
                            gender: person.gender.clone(),
                            country: person.country.clone(),
                            bio: person.bio.clone(),
                            otherids: Some(otherids),
                        }, requesting_user).await?;
                        store.add_book_person(&new_book.id, &created.id, None).await?;
                    }
                }
            }
        }

        let inserted =
            store
                .get_book(&new_book.id)
                .await?
                .ok_or(SourcesError::UnableToFindMovie(
                    library_id.to_string(),
                    new_book.id.clone(),
                    "add_book".to_string(),
                ))?
                .item;
        self.send_book(BooksMessage {
            library: library_id.to_string(),
            books: vec![BookWithAction {
                action: ElementAction::Added,
                book: inserted.clone(),
            }],
        });

        let mc = self.clone();
        let lib_id = library_id.to_string();
        let bid = inserted.id.clone();
        tokio::spawn(async move {
            let _ = mc.enrich_book_ids(&lib_id, &bid, &ConnectedUser::ServerAdmin).await;
        });

        Ok(inserted)
    }

    pub async fn enrich_book_ids(&self, library_id: &str, book_id: &str, requesting_user: &ConnectedUser) -> RsResult<()> {
        let book = self.get_book(library_id, book_id.to_string(), requesting_user)
            .await?
            .item;
        let ids: RsIds = book.clone().into();
        if ids.as_all_external_ids().is_empty() {
            return Ok(());
        }

        let lookup_query = RsLookupQuery::Book(RsLookupBook {
            name: None,
            author: None,
            ids: Some(ids.clone()),
            page_key: None,
        });
        let mut groups = self.exec_lookup_metadata_grouped(
            lookup_query,
            Some(library_id.to_string()),
            requesting_user,
            None,
            None,
        ).await?;
        merge_result_ids(&mut groups);

        let matched = groups.into_iter()
            .flat_map(|(_, _, r)| r.results)
            .find_map(|result| {
                if let RsLookupMetadataResult::Book(b) = result.metadata {
                    let result_ids: RsIds = b.clone().into();
                    if result_ids.has_common_id(&ids) {
                        Some(b)
                    } else {
                        None
                    }
                } else {
                    None
                }
            });

        if let Some(matched) = matched {
            let mut updates = BookForUpdate::default();
            if book.isbn13.is_none() { updates.isbn13 = matched.isbn13; }
            if book.openlibrary_edition_id.is_none() { updates.openlibrary_edition_id = matched.openlibrary_edition_id; }
            if book.openlibrary_work_id.is_none() { updates.openlibrary_work_id = matched.openlibrary_work_id; }
            if book.google_books_volume_id.is_none() { updates.google_books_volume_id = matched.google_books_volume_id; }
            if book.asin.is_none() { updates.asin = matched.asin; }
            if book.year.is_none() { updates.year = matched.year; }
            if book.overview.is_none() { updates.overview = matched.overview; }
            if updates.has_update() {
                self.update_book(library_id, book_id.to_string(), updates, &ConnectedUser::ServerAdmin).await?;
            }
        }
        Ok(())
    }

    pub async fn update_book(
        &self,
        library_id: &str,
        book_id: String,
        update: BookForUpdate,
        requesting_user: &ConnectedUser,
    ) -> RsResult<Book> {
        requesting_user.check_library_role(library_id, LibraryRole::Admin)?;
        if RsIds::is_id(&book_id) {
            return Err(Error::InvalidIdForAction("udpate".to_string(), book_id).into());
        }
        if !update.has_update() {
            return Ok(self
                .get_book(library_id, book_id, requesting_user)
                .await?
                .item);
        }
        let store = self.store.get_library_store(library_id)?;
        let existing = store
            .get_book(&book_id)
            .await?
            .ok_or(SourcesError::UnableToFindMovie(
                library_id.to_string(),
                book_id.clone(),
                "update_book".to_string(),
            ))?
            .item;
        if update.chapter.is_some() && update.serie_ref.is_none() && existing.serie_ref.is_none() {
            return Err(Error::ServiceError(
                "invalid book".to_string(),
                Some("chapter requires serie_ref".to_string()),
            )
            .into());
        }
        store.update_book(&book_id, update).await?;
        let updated = store
            .get_book(&book_id)
            .await?
            .ok_or(SourcesError::UnableToFindMovie(
                library_id.to_string(),
                book_id,
                "update_book".to_string(),
            ))?
            .item;
        self.send_book(BooksMessage {
            library: library_id.to_string(),
            books: vec![BookWithAction {
                action: ElementAction::Updated,
                book: updated.clone(),
            }],
        });
        Ok(updated)
    }

    pub async fn remove_book(
        &self,
        library_id: &str,
        book_id: &str,
        requesting_user: &ConnectedUser,
    ) -> RsResult<Book> {
        requesting_user.check_library_role(library_id, LibraryRole::Write)?;
        if RsIds::is_id(book_id) {
            return Err(
                Error::InvalidIdForAction("remove".to_string(), book_id.to_string()).into(),
            );
        }
        let store = self.store.get_library_store(library_id)?;
        let existing = store
            .get_book(book_id)
            .await?
            .ok_or(SourcesError::UnableToFindMovie(
                library_id.to_string(),
                book_id.to_string(),
                "remove_book".to_string(),
            ))?
            .item;
        store.remove_book(book_id.to_string()).await?;
        self.add_deleted(
            library_id,
            RsDeleted::book(book_id.to_owned()),
            requesting_user,
        )
        .await?;
        self.send_book(BooksMessage {
            library: library_id.to_string(),
            books: vec![BookWithAction {
                action: ElementAction::Deleted,
                book: existing.clone(),
            }],
        });
        Ok(existing)
    }

    pub fn send_book(&self, message: BooksMessage) {
        self.broadcast_sse(SseEvent::Books(message));
    }

    #[async_recursion]
    pub async fn book_image(
        &self,
        library_id: &str,
        book_id: &str,
        kind: Option<ImageType>,
        size: Option<ImageSize>,
        requesting_user: &ConnectedUser,
    ) -> RsResult<FileStreamResult<AsyncReadPinBox>> {
        let target_kind = kind.unwrap_or(ImageType::Poster);
        let config = EntityImageConfig { folder: ".books", cache_prefix: "book" };
        if RsIds::is_id(book_id) {
            let book_ids: RsIds = book_id.to_string().try_into()?;
            let store = self.store.get_library_store(library_id)?;
            let existing_book = store.get_book_by_external_id(book_ids.clone()).await?;
            if let Some(existing_book) = existing_book {
                return self.book_image(library_id, &existing_book.item.id, Some(target_kind), size, requesting_user).await;
            }
            let lookup_query = RsLookupQuery::Book(RsLookupBook {
                name: None,
                author: None,
                ids: Some(book_ids),
                page_key: None,
            });
            self.serve_cached_entity_image(library_id, book_id, lookup_query, &target_kind, &config, requesting_user).await
        } else {
            self.serve_local_entity_image(
                library_id, book_id, &target_kind, size, &config, requesting_user,
                async { self.refresh_book_image(library_id, book_id, &target_kind, requesting_user).await.map(|_| ()) },
            ).await
        }
    }

    pub async fn get_book_images(
        &self,
        query: RsLookupBook,
        library_id: Option<String>,
        requesting_user: &ConnectedUser,
    ) -> RsResult<Vec<ExternalImage>> {
        self.get_entity_images(RsLookupQuery::Book(query), library_id, requesting_user).await
    }

    pub async fn get_book_image_url(
        &self,
        query: RsLookupBook,
        library_id: Option<String>,
        kind: &ImageType,
        requesting_user: &ConnectedUser,
    ) -> RsResult<Option<rs_plugin_common_interfaces::RsRequest>> {
        self.get_entity_image_url(RsLookupQuery::Book(query), library_id, kind, requesting_user).await
    }

    pub async fn download_book_image(
        &self,
        query: RsLookupBook,
        library_id: Option<String>,
        kind: &ImageType,
        requesting_user: &ConnectedUser,
    ) -> RsResult<AsyncReadPinBox> {
        self.download_entity_image(RsLookupQuery::Book(query), library_id, kind, requesting_user).await
    }

    pub async fn refresh_book_image(
        &self,
        library_id: &str,
        book_id: &str,
        kind: &ImageType,
        requesting_user: &ConnectedUser,
    ) -> RsResult<Book> {
        let book = self
            .get_book(library_id, book_id.to_string(), requesting_user)
            .await?
            .item;
        let ids: RsIds = book.clone().into();
        let lookup_query = RsLookupQuery::Book(RsLookupBook {
            name: Some(book.name.clone()),
            author: None,
            ids: Some(ids),
            page_key: None,
        });
        let reader = self
            .download_entity_image(lookup_query, Some(library_id.to_string()), kind, requesting_user)
            .await?;
        self.update_book_image(library_id, &book.id, kind, reader, &ConnectedUser::ServerAdmin).await?;
        Ok(self.get_book(library_id, book.id, requesting_user).await?.item)
    }

    pub async fn update_book_image(
        &self,
        library_id: &str,
        book_id: &str,
        kind: &ImageType,
        reader: AsyncReadPinBox,
        requesting_user: &ConnectedUser,
    ) -> RsResult<()> {
        requesting_user.check_library_role(library_id, LibraryRole::Write)?;
        if RsIds::is_id(book_id) {
            return Err(Error::InvalidIdForAction(
                "udpate book image".to_string(),
                book_id.to_string(),
            )
            .into());
        }

        let converted =
            convert_image_reader(reader, image::ImageFormat::Avif, Some(60), false).await?;
        let converted_reader = Cursor::new(converted);

        self.update_library_image(
            library_id,
            ".books",
            book_id,
            &Some(kind.clone()),
            &None,
            converted_reader,
            requesting_user,
        )
        .await?;

        let book = self
            .get_book(library_id, book_id.to_string(), requesting_user)
            .await?
            .item;
        self.send_book(BooksMessage {
            library: library_id.to_string(),
            books: vec![BookWithAction {
                action: ElementAction::Updated,
                book,
            }],
        });
        Ok(())
    }
}

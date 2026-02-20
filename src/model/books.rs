use std::io::Cursor;

use async_recursion::async_recursion;
use futures::TryStreamExt;
use nanoid::nanoid;
use rs_plugin_common_interfaces::{
    domain::{rs_ids::RsIds, ItemWithRelations},
    lookup::{RsLookupBook, RsLookupMetadataResult, RsLookupQuery},
    request::RsRequest,
    ExternalImage, ImageType,
};
use serde::{Deserialize, Serialize};
use strum_macros::EnumString;
use tokio::io::AsyncWriteExt;

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
        error::SourcesError, AsyncReadPinBox, FileStreamResult, Source, SourceRead,
    },
    routes::sse::SseEvent,
    tools::image_tools::{convert_image_reader, resize_image_reader, ImageSize},
};

use super::{
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
    fn select_book_image_url(images: Vec<ExternalImage>, kind: &ImageType) -> Option<RsRequest> {
        let first_kind_match = images
            .iter()
            .find(|image| image.kind.as_ref() == Some(kind))
            .map(|image| image.url.clone());
        if first_kind_match.is_some() {
            first_kind_match
        } else {
            images.into_iter().next().map(|image| image.url)
        }
    }

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
                });
                let plugin_results = self
                    .exec_lookup_metadata_grouped(
                        lookup_query,
                        Some(library_id.to_string()),
                        requesting_user,
                        None,
                    )
                    .await?;
                let plugin_book = plugin_results.into_values().flatten().find_map(|result| {
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
        requesting_user: &ConnectedUser,
    ) -> RsResult<Book> {
        requesting_user.check_library_role(library_id, LibraryRole::Write)?;
        let mut new_book = item.item;
        let relations = item.relations;
        Self::validate_book(&new_book)?;

        let ids: RsIds = new_book.clone().into();
        if ids.isbn13.is_some()
            || ids.openlibrary_edition_id.is_some()
            || ids.openlibrary_work_id.is_some()
            || ids.google_books_volume_id.is_some()
            || ids.asin.is_some()
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
                    if let Some(_existing_person) = store.get_person(&person.id).await? {
                        store.add_book_person(&new_book.id, &person.id, None).await?;
                    } else if let Some(found) = store.get_person_by_external_id(person_ids).await? {
                        store.add_book_person(&new_book.id, &found.id, Some(80)).await?;
                    } else if upsert_people {
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
                            otherids: person.otherids.clone(),
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
        Ok(inserted)
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

        if RsIds::is_id(book_id) {
            let book_ids: RsIds = book_id.to_string().try_into()?;
            let store = self.store.get_library_store(library_id)?;
            let existing_book = store.get_book_by_external_id(book_ids.clone()).await?;

            if let Some(existing_book) = existing_book {
                self.book_image(
                    library_id,
                    &existing_book.item.id,
                    Some(target_kind),
                    size,
                    requesting_user,
                )
                .await
            } else {
                // Book not in DB — fetch image from plugin lookup and cache it
                let local_provider = self.library_source_for_library(library_id).await?;
                let image_path = format!(
                    "cache/book-{}-{}.avif",
                    book_id.replace(':', "-"),
                    target_kind
                );

                if !local_provider.exists(&image_path).await {
                    let lookup_query = RsLookupBook {
                        name: None,
                        ids: Some(book_ids),
                    };
                    let image_request = self
                        .get_book_image_url(
                            lookup_query,
                            Some(library_id.to_string()),
                            &target_kind,
                            requesting_user,
                        )
                        .await?
                        .ok_or(crate::Error::NotFound(format!(
                            "Unable to get book image url: {} kind {:?}",
                            book_id, target_kind
                        )))?;
                    let (_, mut writer) = local_provider.get_file_write_stream(&image_path).await?;
                    let image_reader = SourceRead::Request(image_request)
                        .into_reader(
                            Some(library_id),
                            None,
                            None,
                            Some((self.clone(), requesting_user)),
                            None,
                        )
                        .await?;
                    let resized = resize_image_reader(
                        image_reader.stream,
                        ImageSize::Large.to_size(),
                        image::ImageFormat::Avif,
                        Some(70),
                        false,
                    )
                    .await?;
                    writer.write_all(&resized).await?;
                }

                let source = local_provider.get_file(&image_path, None).await?;
                match source {
                    SourceRead::Stream(s) => Ok(s),
                    SourceRead::Request(_) => Err(crate::Error::GenericRedseatError),
                }
            }
        } else {
            if !self
                .has_library_image(
                    library_id,
                    ".books",
                    book_id,
                    Some(target_kind.clone()),
                    requesting_user,
                )
                .await?
            {
                self.refresh_book_image(library_id, book_id, &target_kind, requesting_user)
                    .await?;
            }

            self.library_image(
                library_id,
                ".books",
                book_id,
                Some(target_kind),
                size,
                requesting_user,
            )
            .await
        }
    }

    pub async fn get_book_images(
        &self,
        query: RsLookupBook,
        library_id: Option<String>,
        requesting_user: &ConnectedUser,
    ) -> RsResult<Vec<ExternalImage>> {
        let lookup_query = RsLookupQuery::Book(query);
        println!("Executing book image lookup with query: {:?}", lookup_query);
        let images = match self
            .exec_lookup_images(lookup_query, library_id, requesting_user, None)
            .await
        {
            Ok(images) => images,
            Err(error) => {
                crate::tools::log::log_error(
                    crate::tools::log::LogServiceType::Plugin,
                    format!("book image lookup failed: {:#}", error),
                );
                Vec::new()
            }
        };

        //println!("result: {:?}", images);
        Ok(images)
    }

    pub async fn get_book_image_url(
        &self,
        query: RsLookupBook,
        library_id: Option<String>,
        kind: &ImageType,
        requesting_user: &ConnectedUser,
    ) -> RsResult<Option<RsRequest>> {
        let images = self
            .get_book_images(query, library_id, requesting_user)
            .await?;
        Ok(Self::select_book_image_url(images, kind))
    }

    pub async fn download_book_image(
        &self,
        query: RsLookupBook,
        library_id: Option<String>,
        kind: &ImageType,
        requesting_user: &ConnectedUser,
    ) -> RsResult<AsyncReadPinBox> {
        let request = self
            .get_book_image_url(query, library_id.clone(), kind, requesting_user)
            .await?
            .ok_or(crate::Error::NotFound(format!(
                "Unable to get book image url for kind: {:?}",
                kind
            )))?;
        let reader = SourceRead::Request(request)
            .into_reader(
                library_id.as_deref(),
                None,
                None,
                Some((self.clone(), requesting_user)),
                None,
            )
            .await?;
        Ok(reader.stream)
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
        let lookup_query = RsLookupBook {
            name: Some(book.name.clone()),
            ids: Some(ids),
        };
        let reader = self
            .download_book_image(
                lookup_query,
                Some(library_id.to_string()),
                kind,
                requesting_user,
            )
            .await?;
        self.update_book_image(
            library_id,
            &book.id,
            kind,
            reader,
            &ConnectedUser::ServerAdmin,
        )
        .await?;
        Ok(self
            .get_book(library_id, book.id, requesting_user)
            .await?
            .item)
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

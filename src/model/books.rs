use std::io::Cursor;

use nanoid::nanoid;
use rs_plugin_common_interfaces::{domain::rs_ids::RsIds, lookup::{RsLookupBook, RsLookupMetadataResult, RsLookupQuery}, ImageType};
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
    plugins::sources::{error::SourcesError, AsyncReadPinBox},
    routes::sse::SseEvent,
    tools::image_tools::{convert_image_reader, ImageSize},
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
    pub async fn get_books(
        &self,
        library_id: &str,
        query: BookQuery,
        requesting_user: &ConnectedUser,
    ) -> RsResult<Vec<Book>> {
        requesting_user.check_library_role(library_id, LibraryRole::Read)?;
        let store = self.store.get_library_store(library_id)?;
        Ok(store.get_books(query).await?)
    }

    pub async fn get_book(
        &self,
        library_id: &str,
        book_id: String,
        requesting_user: &ConnectedUser,
    ) -> RsResult<Book> {
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
                    title: String::new(),
                    author: String::new(),
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
                let plugin_book = plugin_results
                    .into_values()
                    .flatten()
                    .find_map(|result| match result.metadata {
                        RsLookupMetadataResult::Book(book) => Some(book),
                        _ => None,
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
            store.get_book(&book_id).await?.ok_or(
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
    ) -> RsResult<Option<Book>> {
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
        mut new_book: Book,
        requesting_user: &ConnectedUser,
    ) -> RsResult<Book> {
        requesting_user.check_library_role(library_id, LibraryRole::Write)?;
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
                    Error::Duplicate(existing.id.to_owned(), MediaElement::Book(existing)).into(),
                );
            }
        }

        let store = self.store.get_library_store(library_id)?;
        new_book.id = nanoid!();
        store.add_book(new_book.clone()).await?;
        let inserted =
            store
                .get_book(&new_book.id)
                .await?
                .ok_or(SourcesError::UnableToFindMovie(
                    library_id.to_string(),
                    new_book.id.clone(),
                    "add_book".to_string(),
                ))?;
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
            return self.get_book(library_id, book_id, requesting_user).await;
        }
        let store = self.store.get_library_store(library_id)?;
        let existing = store
            .get_book(&book_id)
            .await?
            .ok_or(SourcesError::UnableToFindMovie(
                library_id.to_string(),
                book_id.clone(),
                "update_book".to_string(),
            ))?;
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
            ))?;
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
            ))?;
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

    pub async fn book_image(
        &self,
        library_id: &str,
        book_id: &str,
        kind: Option<ImageType>,
        size: Option<ImageSize>,
        requesting_user: &ConnectedUser,
    ) -> RsResult<crate::plugins::sources::FileStreamResult<AsyncReadPinBox>> {
        let target_kind = kind.unwrap_or(ImageType::Poster);
        if target_kind != ImageType::Poster {
            return Err(
                Error::NotFound("Only poster image type is supported for books".to_string())
                    .into(),
            );
        }

        let resolved_book_id = if RsIds::is_id(book_id) {
            self.get_book(library_id, book_id.to_string(), requesting_user)
                .await?
                .id
        } else {
            book_id.to_string()
        };

        self.library_image(
            library_id,
            ".books",
            &resolved_book_id,
            Some(ImageType::Poster),
            size,
            requesting_user,
        )
        .await
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
        if *kind != ImageType::Poster {
            return Err(
                Error::NotFound("Only poster image type is supported for books".to_string())
                    .into(),
            );
        }
        if RsIds::is_id(book_id) {
            return Err(
                Error::InvalidIdForAction("udpate book image".to_string(), book_id.to_string())
                    .into(),
            );
        }

        let converted = convert_image_reader(reader, image::ImageFormat::Avif, Some(60), false)
            .await?;
        let converted_reader = Cursor::new(converted);

        self.update_library_image(
            library_id,
            ".books",
            book_id,
            &Some(ImageType::Poster),
            &None,
            converted_reader,
            requesting_user,
        )
        .await?;

        let book = self
            .get_book(library_id, book_id.to_string(), requesting_user)
            .await?;
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

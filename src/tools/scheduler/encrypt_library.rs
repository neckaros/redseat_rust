use crate::{
    domain::library::LibraryStatusMessage,
    error::{RsError, RsResult},
    model::{users::ConnectedUser, ModelController},
    plugins::sources::Source,
    tools::{
        encryption::{derive_key, CtrDecryptReader, CtrEncryptWriter, CTR_NONCE_SIZE},
        log::{log_error, log_info, LogServiceType},
    },
};
use axum::async_trait;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tokio::{
    fs,
    io::{copy, AsyncWriteExt, BufReader, BufWriter},
};

use super::RsSchedulerTask;

const BATCH_SIZE: usize = 50;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct EncryptLibraryTask {
    pub library_id: String,
    /// If true, decrypt files (removing encryption) instead of encrypting
    pub decrypt: bool,
}

impl EncryptLibraryTask {
    pub fn new_encrypt(library_id: String) -> Self {
        Self {
            library_id,
            decrypt: false,
        }
    }

    pub fn new_decrypt(library_id: String) -> Self {
        Self {
            library_id,
            decrypt: true,
        }
    }
}

#[async_trait]
impl RsSchedulerTask for EncryptLibraryTask {
    async fn execute(&self, mc: ModelController) -> RsResult<()> {
        let connected_user = &ConnectedUser::ServerAdmin;
        let library = mc
            .get_library(&self.library_id, connected_user)
            .await?
            .ok_or_else(|| RsError::Error(format!("Library {} not found", self.library_id)))?;

        let password = match &library.password {
            Some(p) => p.clone(),
            None => {
                log_info(
                    LogServiceType::Scheduler,
                    format!("No password for library {}, skipping encryption task", self.library_id),
                );
                return Ok(());
            }
        };

        let key = derive_key(password);

        let source = mc.source_for_library(&self.library_id).await?;

        mc.send_library_status(LibraryStatusMessage {
            library: self.library_id.clone(),
            message: if self.decrypt {
                "Decrypting library files...".to_string()
            } else {
                "Encrypting library files...".to_string()
            },
        });

        // Get all (media_id, source) pairs so we can update sources after re-upload
        let all_media = mc.get_all_media_id_sources(&self.library_id).await?;
        let total = all_media.len();
        let mut processed = 0u64;
        let mut errors = 0u64;

        for (i, (media_id, media_source)) in all_media.iter().enumerate() {
            if let Some(local_path) = source.local_path(media_source) {
                // Local file: encrypt/decrypt in-place via temp file
                let result = if self.decrypt {
                    Self::decrypt_local_file(&local_path, &key).await
                } else {
                    Self::encrypt_local_file(&local_path, &key).await
                };
                match result {
                    Ok(true) => processed += 1,
                    Ok(false) => {}
                    Err(e) => {
                        errors += 1;
                        log_error(
                            LogServiceType::Scheduler,
                            format!("Error processing local file {}: {:?}", media_source, e),
                        );
                    }
                }
            } else {
                // Remote/plugin file: download → encrypt/decrypt → re-upload → update DB source
                let result = Self::process_remote_file(
                    &mc,
                    &self.library_id,
                    media_id,
                    media_source,
                    &key,
                    self.decrypt,
                )
                .await;
                match result {
                    Ok(true) => processed += 1,
                    Ok(false) => {}
                    Err(e) => {
                        errors += 1;
                        log_error(
                            LogServiceType::Scheduler,
                            format!("Error processing remote file {} (media {}): {:?}", media_source, media_id, e),
                        );
                    }
                }
            }

            // Send progress every BATCH_SIZE files
            if (i + 1) % BATCH_SIZE == 0 || i + 1 == total {
                mc.send_library_status(LibraryStatusMessage {
                    library: self.library_id.clone(),
                    message: format!(
                        "{} library files... ({}/{}, {} errors)",
                        if self.decrypt { "Decrypting" } else { "Encrypting" },
                        i + 1,
                        total,
                        errors,
                    ),
                });
            }
        }

        // Process thumbs and portraits folders (always stored locally via PathProvider)
        let local_provider = mc.library_source_for_library(&self.library_id).await
            .map_err(|e| RsError::Error(format!("Failed to get local provider: {:?}", e)))?;
        if let Some(root_path) = local_provider.local_path("") {
            for folder in &[".thumbs", ".portraits", ".series", ".books", ".faces"] {
                let folder_path = root_path.join(folder);
                if folder_path.exists() {
                    let result = Self::process_folder(&folder_path, &key, self.decrypt).await;
                    match result {
                        Ok(count) => {
                            processed += count;
                            log_info(
                                LogServiceType::Scheduler,
                                format!("Processed {} files in {}", count, folder),
                            );
                        }
                        Err(e) => {
                            log_error(
                                LogServiceType::Scheduler,
                                format!("Error processing folder {}: {:?}", folder, e),
                            );
                        }
                    }
                }
            }
        }

        mc.send_library_status(LibraryStatusMessage {
            library: self.library_id.clone(),
            message: format!(
                "Library {} complete. {} files processed, {} errors.",
                if self.decrypt { "decryption" } else { "encryption" },
                processed,
                errors,
            ),
        });

        log_info(
            LogServiceType::Scheduler,
            format!(
                "Library {} {} complete. {} files processed, {} errors.",
                self.library_id,
                if self.decrypt { "decryption" } else { "encryption" },
                processed,
                errors,
            ),
        );

        Ok(())
    }
}

impl EncryptLibraryTask {
    /// Encrypt a local file in-place using a temp file + rename.
    async fn encrypt_local_file(path: &PathBuf, key: &[u8; 32]) -> RsResult<bool> {
        if !path.exists() {
            return Ok(false);
        }

        let temp_path = path.with_extension("encrypting_tmp");

        let input = fs::File::open(&path).await?;
        let reader = BufReader::new(input);

        let output = fs::File::create(&temp_path).await?;
        let writer = BufWriter::new(output);

        let mut enc_writer = CtrEncryptWriter::new(writer, key)?;
        let mut reader = reader;
        copy(&mut reader, &mut enc_writer).await?;
        enc_writer.flush().await?;
        enc_writer.shutdown().await?;

        // Atomic-ish replace: rename temp over original
        fs::rename(&temp_path, &path).await?;

        Ok(true)
    }

    /// Decrypt a local file in-place using a temp file + rename.
    async fn decrypt_local_file(path: &PathBuf, key: &[u8; 32]) -> RsResult<bool> {
        if !path.exists() {
            return Ok(false);
        }

        let temp_path = path.with_extension("decrypting_tmp");

        let input = fs::File::open(&path).await?;
        let reader = BufReader::new(input);

        let output = fs::File::create(&temp_path).await?;
        let mut writer = BufWriter::new(output);

        let mut dec_reader = CtrDecryptReader::new(reader, key);
        copy(&mut dec_reader, &mut writer).await?;
        writer.flush().await?;
        writer.shutdown().await?;

        fs::rename(&temp_path, &path).await?;

        Ok(true)
    }

    /// Process a remote/plugin file: download, encrypt/decrypt, re-upload, update DB source.
    async fn process_remote_file(
        mc: &ModelController,
        library_id: &str,
        media_id: &str,
        old_source: &str,
        key: &[u8; 32],
        decrypt: bool,
    ) -> RsResult<bool> {
        let source_provider = mc.source_for_library(library_id).await?;

        // Download the file
        let file_read = source_provider.get_file(old_source, None).await?;
        let file_stream = file_read
            .into_reader(
                Some(library_id),
                None,
                None,
                Some((mc.clone(), &ConnectedUser::ServerAdmin)),
                None,
            )
            .await?;

        let original_size = file_stream.size;
        let mime = file_stream.mime.clone();

        // Compute the upload size hint accounting for nonce overhead
        let upload_size = if decrypt {
            // Decrypting: output is smaller (remove nonce)
            original_size.map(|s| s.saturating_sub(CTR_NONCE_SIZE))
        } else {
            // Encrypting: output is larger (add nonce)
            original_size.map(|s| s + CTR_NONCE_SIZE)
        };

        // Create a new upload writer via the source provider
        let (new_source_future, mut writer) = source_provider
            .writer(old_source, upload_size, mime)
            .await?;

        // Stream through encrypt/decrypt and write to the new destination
        let mut stream = file_stream.stream;
        if decrypt {
            let mut dec_reader = CtrDecryptReader::new(stream, key);
            copy(&mut dec_reader, &mut writer).await?;
            writer.flush().await?;
            writer.shutdown().await?;
        } else {
            let mut enc_writer = CtrEncryptWriter::new(writer, key)?;
            copy(&mut stream, &mut enc_writer).await?;
            enc_writer.flush().await?;
            enc_writer.shutdown().await?;
        }

        let new_source = new_source_future.await??;

        // Update the media record to point to the new source
        if new_source != old_source {
            mc.update_media_source(library_id, media_id, &new_source).await?;
            // Remove the old file from the provider
            if let Err(e) = source_provider.remove(old_source).await {
                log_error(
                    LogServiceType::Scheduler,
                    format!("Warning: could not remove old source {}: {:?}", old_source, e),
                );
            }
        }

        Ok(true)
    }

    /// Process all files in a directory, recursing into subdirectories.
    fn process_folder<'a>(folder: &'a PathBuf, key: &'a [u8; 32], decrypt: bool) -> std::pin::Pin<Box<dyn std::future::Future<Output = RsResult<u64>> + Send + 'a>> {
        Box::pin(async move {
        let mut count = 0u64;
        let mut entries = fs::read_dir(folder).await?;
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.is_dir() {
                // Recurse into subdirectories (e.g. .series/{serie_id}/)
                match Self::process_folder(&path, key, decrypt).await {
                    Ok(sub_count) => count += sub_count,
                    Err(e) => {
                        log_error(
                            LogServiceType::Scheduler,
                            format!("Error processing subfolder {}: {:?}", path.display(), e),
                        );
                    }
                }
            } else if path.is_file() {
                // Skip temp files
                if let Some(ext) = path.extension() {
                    if ext == "encrypting_tmp" || ext == "decrypting_tmp" {
                        continue;
                    }
                }
                let result = if decrypt {
                    Self::decrypt_local_file(&path, key).await
                } else {
                    Self::encrypt_local_file(&path, key).await
                };
                match result {
                    Ok(true) => count += 1,
                    Ok(false) => {}
                    Err(e) => {
                        log_error(
                            LogServiceType::Scheduler,
                            format!("Error processing {}: {:?}", path.display(), e),
                        );
                    }
                }
            }
        }
        Ok(count)
        })
    }
}

use crate::{
    domain::{library::LibraryStatusMessage, ElementAction},
    error::{RsError, RsResult},
    model::{
        libraries::{LibraryEncryptionItem, LibraryEncryptionJob},
        users::ConnectedUser,
        ModelController,
    },
    plugins::sources::{AsyncReadPinBox, Source},
    tools::{
        encryption::{derive_key, CtrDecryptReader, CtrEncryptWriter, CTR_NONCE_SIZE},
        get_time,
        log::{log_error, LogServiceType},
    },
};
use axum::async_trait;
use nanoid::nanoid;
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, path::PathBuf, pin::Pin};
use tokio::{
    fs,
    io::{copy, AsyncWrite, AsyncWriteExt, BufReader, BufWriter},
};

use super::{RsSchedulerTask, RsSchedulerWhen, RsTaskType};

const RETRY_DELAY_SECONDS: u64 = 60;
const LOCAL_FOLDERS: &[&str] = &[
    ".thumbs",
    ".portraits",
    ".series",
    ".books",
    ".faces",
    "cache",
];

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct EncryptLibraryTask {
    pub job_id: String,
    pub library_id: String,
}

impl EncryptLibraryTask {
    pub fn new(job_id: String, library_id: String) -> Self {
        Self { job_id, library_id }
    }
}

#[async_trait]
impl RsSchedulerTask for EncryptLibraryTask {
    async fn execute(&self, mc: ModelController) -> RsResult<()> {
        match self.run(&mc).await {
            Ok(()) => Ok(()),
            Err(error) => {
                let message = format!("{error:#}");
                log_error(
                    LogServiceType::Scheduler,
                    format!(
                        "Library encryption job {} for {} will retry: {}",
                        self.job_id, self.library_id, message
                    ),
                );
                mc.store
                    .set_library_encryption_error(&self.job_id, Some(message.clone()))
                    .await?;
                mc.send_library_status(LibraryStatusMessage {
                    library: self.library_id.clone(),
                    message: format!("encryption-retry: {message}"),
                });
                mc.scheduler
                    .add(
                        RsTaskType::EncryptLibrary,
                        RsSchedulerWhen::At(
                            get_time().as_secs().saturating_add(RETRY_DELAY_SECONDS),
                        ),
                        self.clone(),
                    )
                    .await?;
                Ok(())
            }
        }
    }
}

impl EncryptLibraryTask {
    async fn run(&self, mc: &ModelController) -> RsResult<()> {
        let Some(mut job) = mc
            .store
            .get_library_encryption_job(&self.library_id)
            .await?
        else {
            return Ok(());
        };
        if job.id != self.job_id || !job.is_active() {
            return Ok(());
        }
        mc.store.set_library_encryption_error(&job.id, None).await?;

        if !job.snapshot_complete {
            let items = Self::snapshot_items(mc, &job).await?;
            mc.store
                .set_library_encryption_snapshot(&job.id, items)
                .await?;
            job = mc
                .store
                .get_library_encryption_job(&self.library_id)
                .await?
                .ok_or_else(|| RsError::Error("Encryption job disappeared".to_string()))?;
        }

        if job.phase == "running" {
            mc.send_library_status(LibraryStatusMessage {
                library: self.library_id.clone(),
                message: "encryption-running".to_string(),
            });
            let mut items = mc.store.list_library_encryption_items(&job.id).await?;
            for item in &mut items {
                if item.state == "pending" {
                    item.staged_source = Some(Self::prepare_item(mc, &job, item).await?);
                    item.state = "prepared".to_string();
                }
                if item.state == "prepared" {
                    Self::commit_item(mc, &job, item).await?;
                } else if item.state == "committed" {
                    Self::cleanup_committed_item(mc, &job, item).await?;
                }
            }

            mc.store
                .set_library_password(&job.library_id, job.target_password.clone())
                .await?;
            let library = mc
                .store
                .get_library(&job.library_id)
                .await?
                .ok_or_else(|| RsError::Error("Library disappeared during encryption".into()))?;
            mc.cache_update_library(library.clone()).await;
            mc.store.complete_library_encryption_job(&job.id).await?;
            mc.send_library(crate::domain::library::LibraryMessage {
                action: ElementAction::Updated,
                library,
            });
            mc.send_library_status(LibraryStatusMessage {
                library: self.library_id.clone(),
                message: "encryption-completed".to_string(),
            });
        }
        Ok(())
    }

    async fn snapshot_items(
        mc: &ModelController,
        job: &LibraryEncryptionJob,
    ) -> RsResult<Vec<LibraryEncryptionItem>> {
        let source = mc.source_for_library_unchecked(&job.library_id).await?;
        let mut items = Vec::new();
        let mut local_paths = HashSet::new();
        let mut remote_sources = HashSet::new();

        for (media_id, media_source) in mc.get_all_media_id_sources(&job.library_id).await? {
            if let Some(path) = source.local_path(&media_source) {
                let path = path.to_string_lossy().into_owned();
                if local_paths.insert(path.clone()) {
                    items.push(Self::item(&job.id, "local", Some(media_id), path));
                }
            } else if remote_sources.insert(media_source.clone()) {
                items.push(Self::item(&job.id, "remote", Some(media_id), media_source));
            }
        }

        let local = mc
            .library_source_for_library_unchecked(&job.library_id)
            .await?;
        if let Some(root) = local.local_path("") {
            for folder in LOCAL_FOLDERS {
                for path in Self::files_below(root.join(folder)).await? {
                    let path = path.to_string_lossy().into_owned();
                    if local_paths.insert(path.clone()) {
                        items.push(Self::item(&job.id, "local", None, path));
                    }
                }
            }
        }
        Ok(items)
    }

    fn item(
        job_id: &str,
        kind: &str,
        media_id: Option<String>,
        source: String,
    ) -> LibraryEncryptionItem {
        LibraryEncryptionItem {
            id: nanoid!(),
            job_id: job_id.to_string(),
            kind: kind.to_string(),
            media_id,
            source,
            staged_source: None,
            state: "pending".to_string(),
        }
    }

    async fn files_below(root: PathBuf) -> RsResult<Vec<PathBuf>> {
        let mut result = Vec::new();
        let mut pending = vec![root];
        while let Some(folder) = pending.pop() {
            let mut entries = match fs::read_dir(&folder).await {
                Ok(entries) => entries,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error.into()),
            };
            while let Some(entry) = entries.next_entry().await? {
                let path = entry.path();
                if path.is_dir() {
                    pending.push(path);
                } else if path.is_file() && !path.to_string_lossy().contains(".redseat-encryption-")
                {
                    result.push(path);
                }
            }
        }
        Ok(result)
    }

    async fn prepare_item(
        mc: &ModelController,
        job: &LibraryEncryptionJob,
        item: &LibraryEncryptionItem,
    ) -> RsResult<String> {
        match item.kind.as_str() {
            "local" => Self::prepare_local(mc, job, item).await,
            "remote" => Self::prepare_remote(mc, job, item).await,
            kind => Err(RsError::Error(format!(
                "Unknown library encryption item kind {kind}"
            ))),
        }
    }

    async fn prepare_local(
        mc: &ModelController,
        job: &LibraryEncryptionJob,
        item: &LibraryEncryptionItem,
    ) -> RsResult<String> {
        let path = PathBuf::from(&item.source);
        if !path.exists() {
            return Err(RsError::Error(format!(
                "Encryption source file is missing: {}",
                path.display()
            )));
        }
        let staged = Self::local_stage_path(&path, &job.id, &item.id);
        if staged.exists() {
            fs::remove_file(&staged).await?;
        }
        let input: AsyncReadPinBox = Box::pin(BufReader::new(fs::File::open(&path).await?));
        let output: Pin<Box<dyn AsyncWrite + Send>> =
            Box::pin(BufWriter::new(fs::File::create(&staged).await?));
        Self::transform(input, output, job).await?;
        let staged = staged.to_string_lossy().into_owned();
        mc.store
            .mark_library_encryption_item_prepared(&item.id, &staged)
            .await?;
        Ok(staged)
    }

    async fn prepare_remote(
        mc: &ModelController,
        job: &LibraryEncryptionJob,
        item: &LibraryEncryptionItem,
    ) -> RsResult<String> {
        let source = mc.source_for_library_unchecked(&job.library_id).await?;
        let source_read = source.get_file(&item.source, None).await?;
        let file = source_read
            .into_reader(
                Some(&job.library_id),
                None,
                None,
                Some((mc.clone(), &ConnectedUser::ServerAdmin)),
                None,
            )
            .await?;
        let output_size = Self::transformed_size(
            file.size,
            job.source_password.is_some(),
            job.target_password.is_some(),
        );
        let (staged_source, writer) = source
            .writer(&item.source, output_size, file.mime.clone())
            .await?;
        Self::transform(file.stream, writer, job).await?;
        let staged_source = staged_source.await??;
        if staged_source == item.source {
            return Err(RsError::Error(
                "Source provider cannot stage encryption uploads under a new identifier".into(),
            ));
        }
        mc.store
            .mark_library_encryption_item_prepared(&item.id, &staged_source)
            .await?;
        Ok(staged_source)
    }

    fn transformed_size(size: Option<u64>, source: bool, target: bool) -> Option<u64> {
        size.map(|size| {
            let plaintext = if source {
                size.saturating_sub(CTR_NONCE_SIZE)
            } else {
                size
            };
            if target {
                plaintext.saturating_add(CTR_NONCE_SIZE)
            } else {
                plaintext
            }
        })
    }

    async fn transform(
        mut input: AsyncReadPinBox,
        output: Pin<Box<dyn AsyncWrite + Send>>,
        job: &LibraryEncryptionJob,
    ) -> RsResult<()> {
        if let Some(password) = &job.source_password {
            input = Box::pin(CtrDecryptReader::new(input, &derive_key(password.clone())));
        }
        if let Some(password) = &job.target_password {
            let mut writer = CtrEncryptWriter::new(output, &derive_key(password.clone()))?;
            copy(&mut input, &mut writer).await?;
            writer.flush().await?;
            writer.shutdown().await?;
        } else {
            let mut output = output;
            copy(&mut input, &mut output).await?;
            output.flush().await?;
            output.shutdown().await?;
        }
        Ok(())
    }

    async fn commit_item(
        mc: &ModelController,
        job: &LibraryEncryptionJob,
        item: &LibraryEncryptionItem,
    ) -> RsResult<()> {
        match item.kind.as_str() {
            "local" => Self::commit_local(job, item).await?,
            "remote" => Self::commit_remote(mc, job, item).await?,
            kind => {
                return Err(RsError::Error(format!(
                    "Unknown library encryption item kind {kind}"
                )))
            }
        }
        mc.store
            .mark_library_encryption_item_committed(&item.id)
            .await?;
        Self::cleanup_committed_item(mc, job, item).await?;
        Ok(())
    }

    async fn commit_local(
        job: &LibraryEncryptionJob,
        item: &LibraryEncryptionItem,
    ) -> RsResult<()> {
        let original = PathBuf::from(&item.source);
        let staged = item
            .staged_source
            .as_ref()
            .map(PathBuf::from)
            .unwrap_or_else(|| Self::local_stage_path(&original, &job.id, &item.id));
        let backup = Self::local_backup_path(&original, &job.id, &item.id);

        if staged.exists() && original.exists() && !backup.exists() {
            fs::rename(&original, &backup).await?;
        }
        if staged.exists() && !original.exists() && backup.exists() {
            fs::rename(&staged, &original).await?;
        }
        if !staged.exists() && original.exists() && backup.exists() {
            return Ok(());
        }
        if staged.exists() && original.exists() && backup.exists() {
            return Err(RsError::Error(format!(
                "Ambiguous local encryption checkpoint for {}",
                original.display()
            )));
        }
        if !original.exists() {
            return Err(RsError::Error(format!(
                "Unable to recover local encryption checkpoint for {}",
                original.display()
            )));
        }
        Ok(())
    }

    async fn commit_remote(
        mc: &ModelController,
        job: &LibraryEncryptionJob,
        item: &LibraryEncryptionItem,
    ) -> RsResult<()> {
        let media_id = item
            .media_id
            .as_deref()
            .ok_or_else(|| RsError::Error("Remote encryption item has no media id".into()))?;
        let staged = item
            .staged_source
            .as_deref()
            .ok_or_else(|| RsError::Error("Remote encryption item was not staged".into()))?;
        let store = mc.store.get_library_store(&job.library_id)?;
        let updated = store.update_media_sources(&item.source, staged).await?;
        if updated == 0
            && store
                .get_media_source(media_id)
                .await?
                .is_none_or(|current| current.source != staged)
        {
            return Err(RsError::Error(format!(
                "Media {media_id} source changed during encryption migration"
            )));
        }
        Ok(())
    }

    async fn cleanup_committed_item(
        mc: &ModelController,
        job: &LibraryEncryptionJob,
        item: &LibraryEncryptionItem,
    ) -> RsResult<()> {
        if item.kind == "local" {
            let backup =
                Self::local_backup_path(&PathBuf::from(&item.source), &job.id, &item.id);
            if backup.exists() {
                fs::remove_file(&backup).await?;
            }
        } else if item.kind == "remote"
            && item.staged_source.as_deref() != Some(item.source.as_str())
        {
            let source = mc.source_for_library_unchecked(&job.library_id).await?;
            if let Err(error) = source.remove(&item.source).await {
                if !matches!(
                    error,
                    RsError::Source(
                        crate::plugins::sources::error::SourcesError::NotFound(_)
                    )
                ) {
                    return Err(error);
                }
            }
        }
        Ok(())
    }

    fn local_stage_path(path: &std::path::Path, job_id: &str, item_id: &str) -> PathBuf {
        Self::local_checkpoint_path(path, job_id, item_id, "stage")
    }

    fn local_backup_path(path: &std::path::Path, job_id: &str, item_id: &str) -> PathBuf {
        Self::local_checkpoint_path(path, job_id, item_id, "backup")
    }

    fn local_checkpoint_path(
        path: &std::path::Path,
        job_id: &str,
        item_id: &str,
        suffix: &str,
    ) -> PathBuf {
        let filename = format!(".redseat-encryption-{job_id}-{item_id}.{suffix}");
        path.parent()
            .map(|parent| parent.join(&filename))
            .unwrap_or_else(|| PathBuf::from(filename))
    }
}

#[cfg(test)]
mod tests {
    use super::EncryptLibraryTask;
    use crate::model::libraries::{LibraryEncryptionItem, LibraryEncryptionJob};
    use crate::tools::encryption::CTR_NONCE_SIZE;
    use std::path::Path;
    use tokio::fs;
    use tokio::io::{duplex, AsyncReadExt};

    #[test]
    fn transformed_size_accounts_for_both_encryption_layers() {
        assert_eq!(
            EncryptLibraryTask::transformed_size(Some(100), false, true),
            Some(100 + CTR_NONCE_SIZE)
        );
        assert_eq!(
            EncryptLibraryTask::transformed_size(Some(100), true, false),
            Some(100 - CTR_NONCE_SIZE)
        );
        assert_eq!(
            EncryptLibraryTask::transformed_size(Some(100), true, true),
            Some(100)
        );
    }

    #[test]
    fn local_checkpoints_use_short_names_in_the_original_directory() {
        let original = Path::new("/media").join(format!("{}.mkv", "x".repeat(240)));
        let staged = EncryptLibraryTask::local_stage_path(&original, "job", "item");
        let backup = EncryptLibraryTask::local_backup_path(&original, "job", "item");

        assert_eq!(staged.parent(), original.parent());
        assert_eq!(backup.parent(), original.parent());
        assert_eq!(staged.file_name().unwrap(), ".redseat-encryption-job-item.stage");
        assert_eq!(backup.file_name().unwrap(), ".redseat-encryption-job-item.backup");
    }

    #[tokio::test]
    async fn local_commit_resumes_at_each_rename_checkpoint() {
        let job = LibraryEncryptionJob {
            id: "job".into(),
            library_id: "library".into(),
            source_password: None,
            target_password: Some("password".into()),
            phase: "running".into(),
            snapshot_complete: true,
            total_items: 1,
            completed_items: 0,
            last_error: None,
        };

        for checkpoint in 0..=2 {
            let directory = tempfile::tempdir().unwrap();
            let original = directory.path().join("movie.mkv");
            let staged = EncryptLibraryTask::local_stage_path(&original, &job.id, "item");
            let backup = EncryptLibraryTask::local_backup_path(&original, &job.id, "item");
            fs::write(&original, b"old").await.unwrap();
            fs::write(&staged, b"new").await.unwrap();
            if checkpoint >= 1 {
                fs::rename(&original, &backup).await.unwrap();
            }
            if checkpoint >= 2 {
                fs::rename(&staged, &original).await.unwrap();
            }

            let item = LibraryEncryptionItem {
                id: "item".into(),
                job_id: job.id.clone(),
                kind: "local".into(),
                media_id: None,
                source: original.to_string_lossy().into_owned(),
                staged_source: Some(staged.to_string_lossy().into_owned()),
                state: "prepared".into(),
            };
            EncryptLibraryTask::commit_local(&job, &item)
                .await
                .unwrap();

            assert_eq!(fs::read(&original).await.unwrap(), b"new");
            assert_eq!(fs::read(&backup).await.unwrap(), b"old");
            assert!(!staged.exists());
        }
    }

    #[tokio::test]
    async fn transform_rekeys_via_plaintext() {
        let plaintext = b"redseat durable encryption".to_vec();
        let encrypt_job = LibraryEncryptionJob {
            id: "job".into(),
            library_id: "library".into(),
            source_password: None,
            target_password: Some("old-password".into()),
            phase: "running".into(),
            snapshot_complete: true,
            total_items: 1,
            completed_items: 0,
            last_error: None,
        };
        let (encrypted_writer, mut encrypted_reader) = duplex(1024);
        EncryptLibraryTask::transform(
            Box::pin(std::io::Cursor::new(plaintext.clone())),
            Box::pin(encrypted_writer),
            &encrypt_job,
        )
        .await
        .unwrap();
        let mut encrypted = Vec::new();
        encrypted_reader.read_to_end(&mut encrypted).await.unwrap();

        let rekey_job = LibraryEncryptionJob {
            source_password: Some("old-password".into()),
            target_password: Some("new-password".into()),
            ..encrypt_job.clone()
        };
        let (rekeyed_writer, mut rekeyed_reader) = duplex(1024);
        EncryptLibraryTask::transform(
            Box::pin(std::io::Cursor::new(encrypted)),
            Box::pin(rekeyed_writer),
            &rekey_job,
        )
        .await
        .unwrap();
        let mut rekeyed = Vec::new();
        rekeyed_reader.read_to_end(&mut rekeyed).await.unwrap();

        let decrypt_job = LibraryEncryptionJob {
            source_password: Some("new-password".into()),
            target_password: None,
            ..encrypt_job
        };
        let (plain_writer, mut plain_reader) = duplex(1024);
        EncryptLibraryTask::transform(
            Box::pin(std::io::Cursor::new(rekeyed)),
            Box::pin(plain_writer),
            &decrypt_job,
        )
        .await
        .unwrap();
        let mut decrypted = Vec::new();
        plain_reader.read_to_end(&mut decrypted).await.unwrap();

        assert_eq!(decrypted, plaintext);
    }
}

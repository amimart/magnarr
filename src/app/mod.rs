pub mod download;
pub mod error;
pub mod model;
pub mod torrent;

use std::sync::Arc;
use std::time::Duration;

use tokio_util::sync::CancellationToken;

use crate::app::download::DownloadRepository;
use crate::app::error::AppError;
use crate::app::model::{Download, DownloadStatus, MagnetUri};
use crate::app::torrent::{TorrentClient, TorrentState};

pub struct App {
    repository: Arc<dyn DownloadRepository>,
    torrent_client: Arc<dyn TorrentClient>,
    poll_interval: Duration,
}

impl App {
    pub fn new(
        repository: Arc<dyn DownloadRepository>,
        torrent_client: Arc<dyn TorrentClient>,
        poll_interval: Duration,
    ) -> Self {
        Self {
            repository,
            torrent_client,
            poll_interval,
        }
    }

    /// Submits a new download: persists it as `Queued`, sends the magnet to the
    /// torrent client, then transitions to `Submitted`. If the client rejects the
    /// magnet the record is deleted (rollback) and an error is returned.
    pub async fn download(
        &self,
        magnet_uri: MagnetUri,
        target_dir: String,
    ) -> Result<Download, AppError> {
        if let Some(hash) = magnet_uri.info_hash() {
            if self.repository.find_by_info_hash(hash)?.is_some() {
                return Err(AppError::AlreadyExists);
            }
        }

        let mut download = Download::new(magnet_uri, target_dir);
        self.repository.create_download(&download)?;

        if let Err(e) = self.torrent_client.download(&download.magnet_uri).await {
            if let Err(del_err) = self.repository.delete_download(download.id) {
                tracing::error!(
                    id = %download.id,
                    "Failed to rollback download after torrent client error: {del_err}"
                );
            }
            return Err(AppError::TorrentClient(e));
        }

        download.status = DownloadStatus::Submitted;
        download.touch();
        self.repository.update_download(&download)?;

        Ok(download)
    }

    /// Spawns a background task that periodically syncs active download
    /// statuses from the torrent client into the repository.
    /// The task exits cleanly when `token` is cancelled.
    pub async fn run(&self, token: CancellationToken) {
        let repository = Arc::clone(&self.repository);
        let torrent_client = Arc::clone(&self.torrent_client);
        let poll_interval = self.poll_interval;

        tokio::spawn(async move {
            loop {
                poll_downloads(&repository, &torrent_client, &token).await;
                tokio::select! {
                    _ = tokio::time::sleep(poll_interval) => {}
                    _ = token.cancelled() => {
                        tracing::info!("Polling loop shut down");
                        break;
                    }
                }
            }
        });
    }
}

async fn poll_downloads(
    repository: &Arc<dyn DownloadRepository>,
    torrent_client: &Arc<dyn TorrentClient>,
    token: &CancellationToken,
) {
    // Pass 1: sync status for actively tracked downloads.
    let mut active = match repository.list_downloads_by_status(DownloadStatus::Submitted) {
        Ok(d) => d,
        Err(e) => {
            tracing::error!("Failed to list Submitted downloads: {e}");
            return;
        }
    };
    match repository.list_downloads_by_status(DownloadStatus::Downloading) {
        Ok(d) => active.extend(d),
        Err(e) => {
            tracing::error!("Failed to list Downloading downloads: {e}");
            return;
        }
    }

    for mut download in active {
        let Some(ref info_hash) = download.info_hash else {
            continue;
        };

        match torrent_client.status(info_hash).await {
            Ok(ts) => {
                if ts.state == TorrentState::Seeding {
                    import_download(repository, &mut download, &ts.save_path, token).await;
                    continue;
                }

                let new_status = match ts.state {
                    TorrentState::Downloading | TorrentState::Paused => DownloadStatus::Downloading,
                    TorrentState::Error => DownloadStatus::Failed,
                    TorrentState::Unknown => download.status,
                    TorrentState::Seeding => unreachable!(),
                };

                if new_status != download.status {
                    download.status = new_status;
                    download.touch();
                    if let Err(e) = repository.update_download(&download) {
                        tracing::error!(id = %download.id, "Failed to update download status: {e}");
                    }
                }
            }
            Err(e) => {
                tracing::warn!(id = %download.id, "Failed to fetch torrent status: {e}");
            }
        }
    }

    // Pass 2: retry any imports that were interrupted by a previous shutdown.
    let importing = match repository.list_downloads_by_status(DownloadStatus::Importing) {
        Ok(d) => d,
        Err(e) => {
            tracing::error!("Failed to list Importing downloads: {e}");
            return;
        }
    };

    for mut download in importing {
        if token.is_cancelled() {
            break;
        }
        let Some(ref info_hash) = download.info_hash else {
            continue;
        };
        match torrent_client.status(info_hash).await {
            Ok(ts) if ts.state == TorrentState::Seeding => {
                import_download(repository, &mut download, &ts.save_path, token).await;
            }
            _ => {} // Not seeding yet — wait for next poll cycle.
        }
    }
}

/// Transitions a download through `Importing` → `Imported` (or `Failed`).
///
/// Copies `save_path` as a named subdirectory under `download.target_dir`.
/// If the token is cancelled mid-copy, the partial destination is cleaned up
/// and the download remains `Importing` so it can be retried on next start.
async fn import_download(
    repository: &Arc<dyn DownloadRepository>,
    download: &mut Download,
    save_path: &str,
    token: &CancellationToken,
) {
    let src = std::path::Path::new(save_path);

    let dir_name = match src.file_name() {
        Some(n) => n.to_owned(),
        None => {
            tracing::error!(id = %download.id, "save_path has no file name component: {save_path}");
            download.status = DownloadStatus::Failed;
            download.error = Some(format!("save_path has no file name: {save_path}"));
            download.touch();
            let _ = repository.update_download(download);
            return;
        }
    };
    let final_dst = std::path::Path::new(&download.target_dir).join(&dir_name);

    download.status = DownloadStatus::Importing;
    download.touch();
    if let Err(e) = repository.update_download(download) {
        tracing::error!(id = %download.id, "Failed to set download to Importing: {e}");
        return;
    }

    match copy_dir_recursive_async(src, &final_dst, token).await {
        Ok(CopyOutcome::Completed) => {
            download.status = DownloadStatus::Imported;
            download.imported_path = Some(final_dst.to_string_lossy().into_owned());
            tracing::info!(id = %download.id, "Download imported to {}", final_dst.display());
        }
        Ok(CopyOutcome::Cancelled) => {
            tracing::info!(id = %download.id, "Import cancelled, cleaning up partial copy");
            if let Err(e) = tokio::fs::remove_dir_all(&final_dst).await {
                tracing::warn!(id = %download.id, "Failed to clean up partial copy at {}: {e}", final_dst.display());
            }
            // Status stays Importing — retry on next start.
            return;
        }
        Err(e) => {
            tracing::error!(id = %download.id, "Failed to copy torrent files: {e}");
            download.status = DownloadStatus::Failed;
            download.error = Some(format!("Import failed: {e}"));
        }
    }

    download.touch();
    if let Err(e) = repository.update_download(download) {
        tracing::error!(id = %download.id, "Failed to update download after import: {e}");
    }
}

enum CopyOutcome {
    Completed,
    Cancelled,
}

async fn copy_dir_recursive_async(
    src: &std::path::Path,
    dst: &std::path::Path,
    token: &CancellationToken,
) -> std::io::Result<CopyOutcome> {
    tokio::fs::create_dir_all(dst).await?;
    let mut rd: tokio::fs::ReadDir = tokio::fs::read_dir(src).await?;
    while let Some(entry) = rd.next_entry().await? {
        if token.is_cancelled() {
            return Ok(CopyOutcome::Cancelled);
        }
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if src_path.is_dir() {
            match Box::pin(copy_dir_recursive_async(&src_path, &dst_path, token)).await? {
                CopyOutcome::Cancelled => return Ok(CopyOutcome::Cancelled),
                CopyOutcome::Completed => {}
            }
        } else {
            tokio::fs::copy(&src_path, &dst_path).await?;
        }
    }
    Ok(CopyOutcome::Completed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::torrent::{TorrentClientError, TorrentStatus};
    use crate::store::redb::RedbStore;
    use async_trait::async_trait;

    const MAGNET: &str = "magnet:?xt=urn:btih:ABCDEF1234567890ABCDEF1234567890ABCDEF12&dn=test";

    struct OkTorrentClient;

    #[async_trait]
    impl TorrentClient for OkTorrentClient {
        async fn download(&self, _magnet: &MagnetUri) -> Result<(), TorrentClientError> {
            Ok(())
        }
        async fn status(&self, info_hash: &str) -> Result<TorrentStatus, TorrentClientError> {
            Err(TorrentClientError::NotFound(info_hash.to_owned()))
        }
    }

    struct FailTorrentClient;

    #[async_trait]
    impl TorrentClient for FailTorrentClient {
        async fn download(&self, _magnet: &MagnetUri) -> Result<(), TorrentClientError> {
            Err(TorrentClientError::Api("simulated failure".to_owned()))
        }
        async fn status(&self, info_hash: &str) -> Result<TorrentStatus, TorrentClientError> {
            Err(TorrentClientError::NotFound(info_hash.to_owned()))
        }
    }

    fn new_app(client: Arc<dyn TorrentClient>) -> (App, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.redb");
        let store = RedbStore::new(path.to_str().unwrap()).unwrap();
        let app = App::new(Arc::new(store), client, Duration::from_secs(60));
        (app, dir)
    }

    #[tokio::test]
    async fn download_happy_path_returns_submitted_download() {
        let (app, _dir) = new_app(Arc::new(OkTorrentClient));
        let magnet: MagnetUri = MAGNET.parse().unwrap();

        let dl = app.download(magnet, "/downloads".to_owned()).await.unwrap();

        assert_eq!(dl.status, DownloadStatus::Submitted);
        assert!(dl.updated_at >= dl.created_at);

        let stored = app.repository.get_download(dl.id).unwrap();
        assert_eq!(stored.status, DownloadStatus::Submitted);
    }

    #[tokio::test]
    async fn download_duplicate_info_hash_returns_already_exists() {
        let (app, _dir) = new_app(Arc::new(OkTorrentClient));
        let magnet: MagnetUri = MAGNET.parse().unwrap();

        app.download(magnet.clone(), "/downloads".to_owned())
            .await
            .unwrap();
        let result = app.download(magnet, "/downloads".to_owned()).await;

        assert!(matches!(result, Err(AppError::AlreadyExists)));
    }

    #[tokio::test]
    async fn download_client_failure_rolls_back_record() {
        let (app, _dir) = new_app(Arc::new(FailTorrentClient));
        let magnet: MagnetUri = MAGNET.parse().unwrap();
        let hash = magnet.info_hash().unwrap().to_owned();

        let result = app.download(magnet, "/downloads".to_owned()).await;

        assert!(matches!(result, Err(AppError::TorrentClient(_))));

        let stored = app.repository.find_by_info_hash(&hash).unwrap();
        assert!(
            stored.is_none(),
            "record should be rolled back on client failure"
        );
    }

    #[tokio::test]
    async fn copy_dir_recursive_async_copies_nested_structure() {
        let src_dir = tempfile::tempdir().unwrap();
        let dst_dir = tempfile::tempdir().unwrap();

        std::fs::write(src_dir.path().join("file.txt"), b"hello").unwrap();
        let sub = src_dir.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(sub.join("nested.txt"), b"world").unwrap();

        let token = CancellationToken::new();
        let result = copy_dir_recursive_async(src_dir.path(), dst_dir.path(), &token)
            .await
            .unwrap();
        assert!(matches!(result, CopyOutcome::Completed));

        assert_eq!(
            std::fs::read(dst_dir.path().join("file.txt")).unwrap(),
            b"hello"
        );
        assert_eq!(
            std::fs::read(dst_dir.path().join("sub/nested.txt")).unwrap(),
            b"world"
        );
    }

    #[tokio::test]
    async fn copy_dir_recursive_async_returns_cancelled_and_leaves_src_intact() {
        let src_dir = tempfile::tempdir().unwrap();
        std::fs::write(src_dir.path().join("file.txt"), b"data").unwrap();

        let token = CancellationToken::new();
        token.cancel(); // already cancelled before we start

        let dst_dir = tempfile::tempdir().unwrap();
        let dst = dst_dir.path().join("output");
        let result = copy_dir_recursive_async(src_dir.path(), &dst, &token)
            .await
            .unwrap();
        assert!(matches!(result, CopyOutcome::Cancelled));
        // dst was created by create_dir_all but no file was copied
        assert!(!dst.join("file.txt").exists());
    }

    #[tokio::test]
    async fn import_download_transitions_to_imported_on_success() {
        let src_dir = tempfile::tempdir().unwrap();
        let dst_dir = tempfile::tempdir().unwrap();
        std::fs::write(src_dir.path().join("movie.mkv"), b"data").unwrap();

        let db_dir = tempfile::tempdir().unwrap();
        let store =
            Arc::new(RedbStore::new(db_dir.path().join("test.redb").to_str().unwrap()).unwrap());
        let uri: MagnetUri = MAGNET.parse().unwrap();
        let mut dl =
            crate::app::model::Download::new(uri, dst_dir.path().to_str().unwrap().to_owned());
        store.create_download(&dl).unwrap();

        let token = CancellationToken::new();
        import_download(
            &(store.clone() as Arc<dyn DownloadRepository>),
            &mut dl,
            src_dir.path().to_str().unwrap(),
            &token,
        )
        .await;

        // dst = target_dir / src_dir_name
        let expected_dst = dst_dir
            .path()
            .join(src_dir.path().file_name().unwrap());

        assert_eq!(dl.status, DownloadStatus::Imported);
        assert_eq!(
            dl.imported_path.as_deref(),
            Some(expected_dst.to_str().unwrap())
        );
        assert!(expected_dst.join("movie.mkv").exists());

        let persisted = store.get_download(dl.id).unwrap();
        assert_eq!(persisted.status, DownloadStatus::Imported);
    }

    #[tokio::test]
    async fn import_download_cancelled_mid_copy_leaves_status_importing() {
        let src_dir = tempfile::tempdir().unwrap();
        std::fs::write(src_dir.path().join("big.mkv"), b"video").unwrap();

        let dst_dir = tempfile::tempdir().unwrap();
        let db_dir = tempfile::tempdir().unwrap();
        let store =
            Arc::new(RedbStore::new(db_dir.path().join("test.redb").to_str().unwrap()).unwrap());
        let uri: MagnetUri = MAGNET.parse().unwrap();
        let mut dl =
            crate::app::model::Download::new(uri, dst_dir.path().to_str().unwrap().to_owned());
        store.create_download(&dl).unwrap();

        let token = CancellationToken::new();
        token.cancel(); // simulate cancellation before copy starts

        import_download(
            &(store.clone() as Arc<dyn DownloadRepository>),
            &mut dl,
            src_dir.path().to_str().unwrap(),
            &token,
        )
        .await;

        assert_eq!(dl.status, DownloadStatus::Importing, "status should remain Importing on cancel");

        // Partial dst must be cleaned up
        let final_dst = dst_dir.path().join(src_dir.path().file_name().unwrap());
        assert!(!final_dst.exists(), "partial dst should be removed on cancel");

        // Status in DB stays Importing (row was updated to Importing before copy started)
        let persisted = store.get_download(dl.id).unwrap();
        assert_eq!(persisted.status, DownloadStatus::Importing);
    }

    #[tokio::test]
    async fn import_download_transitions_to_failed_on_copy_error() {
        let db_dir = tempfile::tempdir().unwrap();
        let store =
            Arc::new(RedbStore::new(db_dir.path().join("test.redb").to_str().unwrap()).unwrap());
        let uri: MagnetUri = MAGNET.parse().unwrap();
        let mut dl = crate::app::model::Download::new(uri, "/nonexistent/target".to_owned());
        store.create_download(&dl).unwrap();

        let token = CancellationToken::new();
        import_download(
            &(store.clone() as Arc<dyn DownloadRepository>),
            &mut dl,
            "/nonexistent/source",
            &token,
        )
        .await;

        assert_eq!(dl.status, DownloadStatus::Failed);
        assert!(dl.error.is_some());

        let persisted = store.get_download(dl.id).unwrap();
        assert_eq!(persisted.status, DownloadStatus::Failed);
    }
}

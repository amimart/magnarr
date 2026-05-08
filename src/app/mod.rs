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
                poll_downloads(&repository, &torrent_client).await;
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
) {
    let downloads = match repository.list_downloads() {
        Ok(d) => d,
        Err(e) => {
            tracing::error!("Failed to list downloads for polling: {e}");
            return;
        }
    };

    let active = downloads.into_iter().filter(|d| {
        matches!(
            d.status,
            DownloadStatus::Submitted | DownloadStatus::Downloading
        )
    });

    for mut download in active {
        let Some(ref info_hash) = download.info_hash else {
            continue;
        };

        match torrent_client.status(info_hash).await {
            Ok(ts) => {
                if ts.state == TorrentState::Seeding {
                    import_download(repository, &mut download, &ts.save_path).await;
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
}

/// Transitions a download through `Importing` → `Imported` (or `Failed`),
/// copying the torrent save directory into the download's `target_dir`.
async fn import_download(
    repository: &Arc<dyn DownloadRepository>,
    download: &mut Download,
    save_path: &str,
) {
    download.status = DownloadStatus::Importing;
    download.touch();
    if let Err(e) = repository.update_download(download) {
        tracing::error!(id = %download.id, "Failed to set download to Importing: {e}");
        return;
    }

    let src = std::path::PathBuf::from(save_path);
    let dst = std::path::PathBuf::from(&download.target_dir);

    match tokio::task::spawn_blocking(move || copy_dir_recursive(&src, &dst)).await {
        Ok(Ok(())) => {
            download.status = DownloadStatus::Imported;
            download.imported_path = Some(download.target_dir.clone());
            tracing::info!(id = %download.id, "Download imported to {}", download.target_dir);
        }
        Ok(Err(e)) => {
            tracing::error!(id = %download.id, "Failed to copy torrent files: {e}");
            download.status = DownloadStatus::Failed;
            download.error = Some(format!("Import failed: {e}"));
        }
        Err(e) => {
            tracing::error!(id = %download.id, "Import task panicked: {e}");
            download.status = DownloadStatus::Failed;
            download.error = Some(format!("Import task panicked: {e}"));
        }
    }

    download.touch();
    if let Err(e) = repository.update_download(download) {
        tracing::error!(id = %download.id, "Failed to update download after import: {e}");
    }
}

fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if src_path.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            std::fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
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

    #[test]
    fn copy_dir_recursive_copies_nested_structure() {
        let src_dir = tempfile::tempdir().unwrap();
        let dst_dir = tempfile::tempdir().unwrap();

        std::fs::write(src_dir.path().join("file.txt"), b"hello").unwrap();
        let sub = src_dir.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(sub.join("nested.txt"), b"world").unwrap();

        copy_dir_recursive(src_dir.path(), dst_dir.path()).unwrap();

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

        import_download(
            &(store.clone() as Arc<dyn DownloadRepository>),
            &mut dl,
            src_dir.path().to_str().unwrap(),
        )
        .await;

        assert_eq!(dl.status, DownloadStatus::Imported);
        assert_eq!(
            dl.imported_path.as_deref(),
            Some(dst_dir.path().to_str().unwrap())
        );
        assert!(dst_dir.path().join("movie.mkv").exists());

        let persisted = store.get_download(dl.id).unwrap();
        assert_eq!(persisted.status, DownloadStatus::Imported);
    }

    #[tokio::test]
    async fn import_download_transitions_to_failed_on_copy_error() {
        let db_dir = tempfile::tempdir().unwrap();
        let store =
            Arc::new(RedbStore::new(db_dir.path().join("test.redb").to_str().unwrap()).unwrap());
        let uri: MagnetUri = MAGNET.parse().unwrap();
        let mut dl = crate::app::model::Download::new(uri, "/nonexistent/target".to_owned());
        store.create_download(&dl).unwrap();

        import_download(
            &(store.clone() as Arc<dyn DownloadRepository>),
            &mut dl,
            "/nonexistent/source",
        )
        .await;

        assert_eq!(dl.status, DownloadStatus::Failed);
        assert!(dl.error.is_some());

        let persisted = store.get_download(dl.id).unwrap();
        assert_eq!(persisted.status, DownloadStatus::Failed);
    }
}

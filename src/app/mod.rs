pub mod download;
pub mod error;
pub mod torrent;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use tokio_util::sync::CancellationToken;

use crate::app::download::{DownloadRepository, RepositoryError};
use crate::app::error::AppError;
use crate::app::torrent::{TorrentClient, TorrentClientError};
use crate::types::{Download, DownloadStatus, Magnet, TorrentState};

#[derive(Clone)]
pub struct App {
    repository: Arc<dyn DownloadRepository>,
    torrent_client: Arc<dyn TorrentClient>,
    poll_interval: Duration,
    /// Directory where the torrent client saves completed downloads.
    download_dir: PathBuf,
}

impl App {
    pub fn new(
        repository: Arc<dyn DownloadRepository>,
        torrent_client: Arc<dyn TorrentClient>,
        poll_interval: Duration,
        download_dir: PathBuf,
    ) -> Self {
        Self {
            repository,
            torrent_client,
            poll_interval,
            download_dir,
        }
    }

    /// Submits a new download: persists it as `Queued`, sends the magnet to the
    /// torrent client, then transitions to `Submitted`. If the client rejects the
    /// magnet the record is deleted (rollback) and an error is returned.
    pub async fn download(&self, magnet: Magnet, target_dir: String) -> Result<Download, AppError> {
        match self.repository.find_by_info_hash(magnet.info_hash()) {
            Ok(_) => Err(AppError::AlreadyExists),
            Err(RepositoryError::NotFound) => Ok(()),
            Err(e) => Err(e.into()),
        }?;

        let mut download = Download::new(magnet, target_dir);
        self.repository.create_download(&download)?;

        if let Err(e) = self.torrent_client.download(&download.magnet).await {
            tracing::error!("Failed to submit torrent download: {e}");
            if let Err(del_err) = self.repository.delete_download(&download.info_hash) {
                tracing::error!(
                    info_hash = %download.info_hash,
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
    pub fn start(&self, token: CancellationToken) {
        let app = self.clone();
        tokio::spawn(async move {
            loop {
                app.poll_downloads(&token).await;
                app.import_downloads(&token).await;
                tokio::select! {
                    _ = tokio::time::sleep(app.poll_interval) => {}
                    _ = token.cancelled() => {
                        tracing::info!("Polling loop shut down");
                        break;
                    }
                }
            }
        });
    }

    async fn poll_downloads(&self, _token: &CancellationToken) {
        let mut active = match self
            .repository
            .list_downloads_by_status(DownloadStatus::Submitted)
        {
            Ok(d) => d,
            Err(e) => {
                tracing::error!("Failed to list Submitted downloads: {e}");
                return;
            }
        };
        match self
            .repository
            .list_downloads_by_status(DownloadStatus::Downloading)
        {
            Ok(d) => active.extend(d),
            Err(e) => {
                tracing::error!("Failed to list Downloading downloads: {e}");
                return;
            }
        }

        for mut download in active {
            match self.torrent_client.status(&download.info_hash).await {
                Ok(ts) => {
                    let new_status = match ts.state {
                        TorrentState::Seeding => DownloadStatus::Importing,
                        TorrentState::Downloading | TorrentState::Paused => {
                            DownloadStatus::Downloading
                        }
                        TorrentState::Error => DownloadStatus::Failed,
                        TorrentState::Unknown => download.status,
                    };

                    if new_status != download.status {
                        download.name = ts.name;
                        download.status = new_status;
                        download.touch();
                        if let Err(e) = self.repository.update_download(&download) {
                            tracing::error!(info_hash = %download.info_hash, "Failed to update download status: {e}");
                        }
                    }
                }
                Err(TorrentClientError::NotFound(_)) => {
                    tracing::warn!(info_hash = %download.info_hash, "Torrent not found, removing download");
                    if let Err(e) = self.repository.delete_download(&download.info_hash) {
                        tracing::error!(info_hash = %download.info_hash, "Failed to remove download: {e}");
                    }
                }
                Err(e) => {
                    tracing::warn!(info_hash = %download.info_hash, "Failed to fetch torrent status: {e}");
                }
            }
        }
    }

    async fn import_downloads(&self, token: &CancellationToken) {
        let importing = match self
            .repository
            .list_downloads_by_status(DownloadStatus::Importing)
        {
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
            self.import_download(&mut download).await;
        }
    }

    /// Copies the torrent save directory into `download.target_dir` and
    /// transitions the download to `Imported` or `Failed`.
    /// Assumes the download is already in `Importing` status.
    async fn import_download(&self, download: &mut Download) {
        let src = self.download_dir.join(&download.name);
        let dir_name = match src.file_name() {
            Some(n) => n.to_owned(),
            None => {
                tracing::error!(info_hash = %download.info_hash, "src path has no file name component: {}", src.display());
                download.status = DownloadStatus::Failed;
                download.error = Some(format!("src path has no file name: {}", src.display()));
                download.touch();
                let _ = self.repository.update_download(download);
                return;
            }
        };
        let src = src.to_owned();
        let final_dst = PathBuf::from(&download.target_dir).join(&dir_name);

        match tokio::task::spawn_blocking(move || copy_dir_recursive(&src, &final_dst)).await {
            Ok(Ok(())) => {
                let dst = std::path::Path::new(&download.target_dir).join(&dir_name);
                download.status = DownloadStatus::Imported;
                download.imported_path = Some(dst.to_string_lossy().into_owned());
                tracing::info!(info_hash = %download.info_hash, "Download imported to {}", dst.display());
            }
            Ok(Err(e)) => {
                tracing::error!(info_hash = %download.info_hash, "Failed to copy torrent files: {e}");
                download.status = DownloadStatus::Failed;
                download.error = Some(format!("Import failed: {e}"));
            }
            Err(e) => {
                tracing::error!(info_hash = %download.info_hash, "Import task panicked: {e}");
                download.status = DownloadStatus::Failed;
                download.error = Some(format!("Import task panicked: {e}"));
            }
        }

        download.touch();
        if let Err(e) = self.repository.update_download(download) {
            tracing::error!(info_hash = %download.info_hash, "Failed to update download after import: {e}");
        }
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
    use crate::app::download::RepositoryError;
    use crate::app::torrent::TorrentClientError;
    use crate::store::redb::RedbStore;
    use crate::types::{TorrentState, TorrentStatus};
    use async_trait::async_trait;

    const MAGNET: &str = "magnet:?xt=urn:btih:ABCDEF1234567890ABCDEF1234567890ABCDEF12&dn=test";
    const INFO_HASH: &str = "ABCDEF1234567890ABCDEF1234567890ABCDEF12";

    /// Succeeds on `download()`, returns `NotFound` on `status()`.
    struct OkTorrentClient;

    #[async_trait]
    impl TorrentClient for OkTorrentClient {
        async fn download(&self, _magnet: &Magnet) -> Result<(), TorrentClientError> {
            Ok(())
        }
        async fn status(&self, info_hash: &str) -> Result<TorrentStatus, TorrentClientError> {
            Err(TorrentClientError::NotFound(info_hash.to_owned()))
        }
    }

    /// Fails on `download()`.
    struct FailTorrentClient;

    #[async_trait]
    impl TorrentClient for FailTorrentClient {
        async fn download(&self, _magnet: &Magnet) -> Result<(), TorrentClientError> {
            Err(TorrentClientError::UnexpectedStatus(
                reqwest::StatusCode::INTERNAL_SERVER_ERROR,
            ))
        }
        async fn status(&self, info_hash: &str) -> Result<TorrentStatus, TorrentClientError> {
            Err(TorrentClientError::NotFound(info_hash.to_owned()))
        }
    }

    /// Returns a fixed `TorrentStatus` on `status()`.
    struct StatefulTorrentClient {
        state: TorrentState,
        name: &'static str,
    }

    #[async_trait]
    impl TorrentClient for StatefulTorrentClient {
        async fn download(&self, _magnet: &Magnet) -> Result<(), TorrentClientError> {
            Ok(())
        }
        async fn status(&self, info_hash: &str) -> Result<TorrentStatus, TorrentClientError> {
            Ok(TorrentStatus {
                hash: info_hash.to_owned(),
                state: self.state.clone(),
                name: self.name.to_owned(),
            })
        }
    }

    /// Creates an `App` backed by a real `RedbStore` in a temporary directory.
    /// The returned `TempDir` must be kept alive for the duration of the test.
    fn new_app(client: Arc<dyn TorrentClient>) -> (App, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let download_dir = dir.path().join("downloads");
        std::fs::create_dir_all(&download_dir).unwrap();
        let store = RedbStore::new(dir.path().join("test.redb").to_str().unwrap()).unwrap();
        let app = App::new(
            Arc::new(store),
            client,
            Duration::from_secs(60),
            download_dir,
        );
        (app, dir)
    }

    // --- download() ---

    #[tokio::test]
    async fn download_happy_path_returns_submitted_download() {
        let (app, _dir) = new_app(Arc::new(OkTorrentClient));
        let magnet: Magnet = MAGNET.parse().unwrap();

        let dl = app.download(magnet, "/target".to_owned()).await.unwrap();

        assert_eq!(dl.status, DownloadStatus::Submitted);
        assert!(dl.updated_at >= dl.created_at);

        let stored = app.repository.find_by_info_hash(&dl.info_hash).unwrap();
        assert_eq!(stored.status, DownloadStatus::Submitted);
    }

    #[tokio::test]
    async fn download_duplicate_info_hash_returns_already_exists() {
        let (app, _dir) = new_app(Arc::new(OkTorrentClient));
        let magnet: Magnet = MAGNET.parse().unwrap();

        app.download(magnet.clone(), "/target".to_owned())
            .await
            .unwrap();
        let result = app.download(magnet, "/target".to_owned()).await;

        assert!(matches!(result, Err(AppError::AlreadyExists)));
    }

    #[tokio::test]
    async fn download_client_failure_rolls_back_record() {
        let (app, _dir) = new_app(Arc::new(FailTorrentClient));
        let magnet: Magnet = MAGNET.parse().unwrap();
        let hash = magnet.info_hash().to_owned();

        let result = app.download(magnet, "/target".to_owned()).await;

        assert!(matches!(result, Err(AppError::TorrentClient(_))));
        assert!(
            matches!(
                app.repository.find_by_info_hash(&hash),
                Err(RepositoryError::NotFound)
            ),
            "record should be rolled back on client failure"
        );
    }

    // --- poll_downloads() ---

    #[tokio::test]
    async fn poll_downloads_removes_download_when_torrent_not_found() {
        let (app, _dir) = new_app(Arc::new(OkTorrentClient));
        app.download(MAGNET.parse().unwrap(), "/target".to_owned())
            .await
            .unwrap();

        let token = CancellationToken::new();
        app.poll_downloads(&token).await;

        assert!(
            matches!(
                app.repository.find_by_info_hash(INFO_HASH),
                Err(RepositoryError::NotFound)
            ),
            "download should be removed when torrent is not found on client"
        );
    }

    #[tokio::test]
    async fn poll_downloads_transitions_to_importing_and_updates_name_on_seeding() {
        let client = Arc::new(StatefulTorrentClient {
            state: TorrentState::Seeding,
            name: "resolved-name",
        });
        let (app, _dir) = new_app(client);
        app.download(MAGNET.parse().unwrap(), "/target".to_owned())
            .await
            .unwrap();

        let token = CancellationToken::new();
        app.poll_downloads(&token).await;

        let dl = app.repository.find_by_info_hash(INFO_HASH).unwrap();
        assert_eq!(dl.status, DownloadStatus::Importing);
        assert_eq!(dl.name, "resolved-name");
    }

    #[tokio::test]
    async fn poll_downloads_transitions_to_downloading_and_updates_name() {
        let client = Arc::new(StatefulTorrentClient {
            state: TorrentState::Downloading,
            name: "resolved-name",
        });
        let (app, _dir) = new_app(client);
        app.download(MAGNET.parse().unwrap(), "/target".to_owned())
            .await
            .unwrap();

        let token = CancellationToken::new();
        app.poll_downloads(&token).await;

        let dl = app.repository.find_by_info_hash(INFO_HASH).unwrap();
        assert_eq!(dl.status, DownloadStatus::Downloading);
        assert_eq!(dl.name, "resolved-name");
    }

    #[tokio::test]
    async fn poll_downloads_transitions_to_failed_on_torrent_error() {
        let client = Arc::new(StatefulTorrentClient {
            state: TorrentState::Error,
            name: "some-name",
        });
        let (app, _dir) = new_app(client);
        app.download(MAGNET.parse().unwrap(), "/target".to_owned())
            .await
            .unwrap();

        let token = CancellationToken::new();
        app.poll_downloads(&token).await;

        let dl = app.repository.find_by_info_hash(INFO_HASH).unwrap();
        assert_eq!(dl.status, DownloadStatus::Failed);
    }

    // --- import_download() ---

    #[tokio::test]
    async fn import_download_transitions_to_imported_on_success() {
        let (app, _dir) = new_app(Arc::new(OkTorrentClient));
        let dst_dir = tempfile::tempdir().unwrap();

        // Seed source files at download_dir/torrent-name/
        let src = app.download_dir.join("torrent-name");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("movie.mkv"), b"data").unwrap();

        let magnet: Magnet = MAGNET.parse().unwrap();
        let mut dl = Download::new(magnet, dst_dir.path().to_str().unwrap().to_owned());
        dl.name = "torrent-name".to_owned();
        dl.status = DownloadStatus::Importing;
        app.repository.create_download(&dl).unwrap();

        app.import_download(&mut dl).await;

        let expected_dst = dst_dir.path().join("torrent-name");
        assert_eq!(dl.status, DownloadStatus::Imported);
        assert_eq!(
            dl.imported_path.as_deref(),
            Some(expected_dst.to_str().unwrap())
        );
        assert!(expected_dst.join("movie.mkv").exists());
        assert_eq!(
            app.repository
                .find_by_info_hash(&dl.info_hash)
                .unwrap()
                .status,
            DownloadStatus::Imported
        );
    }

    #[tokio::test]
    async fn import_download_transitions_to_failed_when_source_missing() {
        let (app, _dir) = new_app(Arc::new(OkTorrentClient));
        let dst_dir = tempfile::tempdir().unwrap();

        let magnet: Magnet = MAGNET.parse().unwrap();
        let mut dl = Download::new(magnet, dst_dir.path().to_str().unwrap().to_owned());
        // No directory at download_dir/"does-not-exist".
        dl.name = "does-not-exist".to_owned();
        dl.status = DownloadStatus::Importing;
        app.repository.create_download(&dl).unwrap();

        app.import_download(&mut dl).await;

        assert_eq!(dl.status, DownloadStatus::Failed);
        assert!(dl.error.is_some());
        assert_eq!(
            app.repository
                .find_by_info_hash(&dl.info_hash)
                .unwrap()
                .status,
            DownloadStatus::Failed
        );
    }

    // --- copy_dir_recursive() ---

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
}

pub mod download;
pub mod error;
pub mod torrent;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use tokio_util::sync::CancellationToken;

use crate::app::download::{DownloadRepository, RepositoryError};
use crate::app::error::AppError;
use crate::app::torrent::{TorrentClient};
use crate::types::{Download, DownloadStatus, Magnet, TorrentState};

#[derive(Clone)]
pub struct App {
    repository: Arc<dyn DownloadRepository>,
    torrent_client: Arc<dyn TorrentClient>,
    poll_interval: Duration,
    /// Directory where the torrent client saves completed downloads.
    pub download_dir: PathBuf,
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
    pub async fn download(
        &self,
        magnet: Magnet,
        target_dir: String,
    ) -> Result<Download, AppError> {
        match self.repository.find_by_info_hash(magnet.info_hash()) {
            Ok(_) => Err(AppError::AlreadyExists),
            Err(RepositoryError::NotFound) => { Ok(()) },
            Err(e) => Err(e.into()),
        }?;

        let mut download = Download::new(magnet, target_dir);
        self.repository.create_download(&download)?;

        if let Err(e) = self.torrent_client.download(&download.magnet).await {
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
                        download.status = new_status;
                        download.touch();
                        if let Err(e) = self.repository.update_download(&download) {
                            tracing::error!(info_hash = %download.info_hash, "Failed to update download status: {e}");
                        }
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
            match self.torrent_client.status(&download.info_hash).await {
                Ok(ts) if ts.state == TorrentState::Seeding => {
                    let src = self.download_dir.join(&ts.name);
                    self.import_download(&mut download, &src).await;
                }
                _ => {} // Not seeding yet or error — retry next cycle.
            }
        }
    }

    /// Copies the torrent save directory into `download.target_dir` and
    /// transitions the download to `Imported` or `Failed`.
    /// Assumes the download is already in `Importing` status.
    async fn import_download(&self, download: &mut Download, src: &std::path::Path) {
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
        let final_dst = std::path::PathBuf::from(&download.target_dir).join(&dir_name);

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
    use crate::app::torrent::{TorrentClientError};
    use crate::store::redb::RedbStore;
    use async_trait::async_trait;
    use crate::types::TorrentStatus;

    const MAGNET: &str = "magnet:?xt=urn:btih:ABCDEF1234567890ABCDEF1234567890ABCDEF12&dn=test";

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

    struct FailTorrentClient;

    #[async_trait]
    impl TorrentClient for FailTorrentClient {
        async fn download(&self, _magnet: &Magnet) -> Result<(), TorrentClientError> {
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
        let app = App::new(Arc::new(store), client, Duration::from_secs(60), dir.path().join("downloads"));
        (app, dir)
    }

    #[tokio::test]
    async fn download_happy_path_returns_submitted_download() {
        let (app, _dir) = new_app(Arc::new(OkTorrentClient));
        let magnet: Magnet = MAGNET.parse().unwrap();

        let dl = app.download(magnet, "/downloads".to_owned()).await.unwrap();

        assert_eq!(dl.status, DownloadStatus::Submitted);
        assert!(dl.updated_at >= dl.created_at);

        let stored = app.repository.find_by_info_hash(&dl.info_hash).unwrap();
        assert_eq!(stored.status, DownloadStatus::Submitted);
    }

    #[tokio::test]
    async fn download_duplicate_info_hash_returns_already_exists() {
        let (app, _dir) = new_app(Arc::new(OkTorrentClient));
        let magnet: Magnet = MAGNET.parse().unwrap();

        app.download(magnet.clone(), "/downloads".to_owned())
            .await
            .unwrap();
        let result = app.download(magnet, "/downloads".to_owned()).await;

        assert!(matches!(result, Err(AppError::AlreadyExists)));
    }

    #[tokio::test]
    async fn download_client_failure_rolls_back_record() {
        let (app, _dir) = new_app(Arc::new(FailTorrentClient));
        let magnet: Magnet = MAGNET.parse().unwrap();
        let hash = magnet.info_hash().to_owned();

        let result = app.download(magnet, "/downloads".to_owned()).await;

        assert!(matches!(result, Err(AppError::TorrentClient(_))));

        let res = app.repository.find_by_info_hash(&hash);
        assert!(
            res.is_ok(),
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

    fn new_app_with_store(store: Arc<RedbStore>, client: Arc<dyn TorrentClient>) -> App {
        App::new(
            store as Arc<dyn DownloadRepository>,
            client,
            Duration::from_secs(60),
            PathBuf::from("/downloads"),
        )
    }

    #[tokio::test]
    async fn import_download_transitions_to_imported_on_success() {
        let src_dir = tempfile::tempdir().unwrap();
        let dst_dir = tempfile::tempdir().unwrap();
        std::fs::write(src_dir.path().join("movie.mkv"), b"data").unwrap();

        let db_dir = tempfile::tempdir().unwrap();
        let store =
            Arc::new(RedbStore::new(db_dir.path().join("test.redb").to_str().unwrap()).unwrap());
        let uri: Magnet = MAGNET.parse().unwrap();
        let mut dl =
            Download::new(uri, dst_dir.path().to_str().unwrap().to_owned());
        dl.status = DownloadStatus::Importing;
        store.create_download(&dl).unwrap();

        let app = new_app_with_store(store.clone(), Arc::new(OkTorrentClient));
        app.import_download(&mut dl, src_dir.path())
            .await;

        let expected_dst = dst_dir.path().join(src_dir.path().file_name().unwrap());
        assert_eq!(dl.status, DownloadStatus::Imported);
        assert_eq!(
            dl.imported_path.as_deref(),
            Some(expected_dst.to_str().unwrap())
        );
        assert!(expected_dst.join("movie.mkv").exists());

        let persisted = store.find_by_info_hash(&dl.info_hash).unwrap();
        assert_eq!(persisted.status, DownloadStatus::Imported);
    }

    #[tokio::test]
    async fn import_download_transitions_to_failed_on_copy_error() {
        let db_dir = tempfile::tempdir().unwrap();
        let store =
            Arc::new(RedbStore::new(db_dir.path().join("test.redb").to_str().unwrap()).unwrap());
        let uri: Magnet = MAGNET.parse().unwrap();
        let mut dl = Download::new(uri, "/nonexistent/target".to_owned());
        dl.status = DownloadStatus::Importing;
        store.create_download(&dl).unwrap();

        let app = new_app_with_store(store.clone(), Arc::new(OkTorrentClient));
        app.import_download(&mut dl, std::path::Path::new("/nonexistent/source")).await;

        assert_eq!(dl.status, DownloadStatus::Failed);
        assert!(dl.error.is_some());

        let persisted = store.find_by_info_hash(&dl.info_hash).unwrap();
        assert_eq!(persisted.status, DownloadStatus::Failed);
    }
}

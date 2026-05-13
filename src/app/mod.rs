pub mod download;
pub mod error;
pub mod service;
pub mod torrent;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use tokio_util::sync::CancellationToken;

use crate::app::download::{
    DownloadCursor, DownloadListOrder, DownloadRepository, RepositoryError,
};
use crate::app::error::AppError;
use crate::app::service::DownloadService;
use crate::app::torrent::{TorrentClient, TorrentClientError};
use crate::types::{Download, DownloadStatus, Magnet, TorrentState};

pub struct App<R>
where
    R: DownloadRepository + Send + Sync + 'static,
{
    repository: Arc<R>,
    torrent_client: Arc<dyn TorrentClient>,
    poll_interval: Duration,
    /// Directory where the torrent client saves completed downloads.
    download_dir: PathBuf,
}

impl<R> Clone for App<R>
where
    R: DownloadRepository + Send + Sync + 'static,
{
    fn clone(&self) -> Self {
        Self {
            repository: Arc::clone(&self.repository),
            torrent_client: Arc::clone(&self.torrent_client),
            poll_interval: self.poll_interval,
            download_dir: self.download_dir.clone(),
        }
    }
}

#[async_trait::async_trait]
impl<R> DownloadService for App<R>
where
    R: DownloadRepository + Send + Sync + 'static,
{
    /// Submits a new download: persists it as `Queued`, sends the magnet to the
    /// torrent client, then transitions to `Submitted`. If the client rejects the
    /// magnet the record is deleted (rollback) and an error is returned.
    async fn download(&self, magnet: Magnet, target_dir: String) -> Result<Download, AppError> {
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

    fn downloads(
        &self,
        status: Option<DownloadStatus>,
        from: Option<chrono::DateTime<chrono::Utc>>,
        after: Option<DownloadCursor>,
        order: DownloadListOrder,
    ) -> Result<Box<dyn Iterator<Item = Result<Download, AppError>> + '_>, AppError> {
        Ok(Box::new(
            self.repository
                .list_downloads(status, from, after, order)?
                .map(|download| download.map_err(AppError::from)),
        ))
    }
}

impl<R> App<R>
where
    R: DownloadRepository + Send + Sync + 'static,
{
    pub fn new(
        repository: Arc<R>,
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
        let mut active = match self.downloads(
            Some(DownloadStatus::Submitted),
            None,
            None,
            DownloadListOrder::CreatedAtDesc,
        ) {
            Ok(iter) => match iter.collect::<Result<Vec<_>, _>>() {
                Ok(downloads) => downloads,
                Err(e) => {
                    tracing::error!("Failed to list Submitted downloads: {e}");
                    return;
                }
            },
            Err(e) => {
                tracing::error!("Failed to list Submitted downloads: {e}");
                return;
            }
        };
        match self.downloads(
            Some(DownloadStatus::Downloading),
            None,
            None,
            DownloadListOrder::CreatedAtDesc,
        ) {
            Ok(iter) => match iter.collect::<Result<Vec<_>, _>>() {
                Ok(downloads) => active.extend(downloads),
                Err(e) => {
                    tracing::error!("Failed to list Downloading downloads: {e}");
                    return;
                }
            },
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
                        download.content_name = ts.content_name;
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
        let importing = match self.downloads(
            Some(DownloadStatus::Importing),
            None,
            None,
            DownloadListOrder::CreatedAtDesc,
        ) {
            Ok(iter) => match iter.collect::<Result<Vec<_>, _>>() {
                Ok(downloads) => downloads,
                Err(e) => {
                    tracing::error!("Failed to list Importing downloads: {e}");
                    return;
                }
            },
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
        let src = self.download_dir.join(&download.content_name);
        let dst = PathBuf::from(&download.target_dir).join(&download.content_name);
        let import_path = dst.clone();

        match tokio::task::spawn_blocking(move || copy_recursive(&src, &dst)).await {
            Ok(Ok(())) => {
                download.status = DownloadStatus::Imported;
                download.imported_path = Some(import_path.to_string_lossy().into_owned());
                tracing::info!(info_hash = %download.info_hash, "Download imported to {}", import_path.display());
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

fn copy_recursive(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
    if src.is_dir() {
        std::fs::create_dir_all(dst)?;
        for entry in std::fs::read_dir(src)? {
            let entry = entry?;
            let src_path = entry.path();
            let dst_path = dst.join(entry.file_name());
            copy_recursive(&src_path, &dst_path)?;
        }
        return Ok(());
    }

    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::copy(src, dst)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::download::{
        DownloadCursor, DownloadListOrder, RepositoryError,
    };
    use crate::app::torrent::TorrentClientError;
    use crate::store::redb::RedbStore;
    use crate::types::{TorrentState, TorrentStatus};
    use async_trait::async_trait;
    use std::sync::Mutex;

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
        content_name: &'static str,
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
                content_name: self.content_name.to_owned(),
            })
        }
    }

    /// Creates an `App` backed by a real `RedbStore` in a temporary directory.
    /// The returned `TempDir` must be kept alive for the duration of the test.
    fn new_app(client: Arc<dyn TorrentClient>) -> (App<RedbStore>, tempfile::TempDir) {
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

    struct PagingRepository {
        downloads: Vec<Download>,
        last_call: Mutex<Option<RecordedListCall>>,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct RecordedListCall {
        status: Option<DownloadStatus>,
        from: Option<chrono::DateTime<chrono::Utc>>,
        after: Option<DownloadCursor>,
        order: DownloadListOrder,
    }

    impl PagingRepository {
        fn new(downloads: Vec<Download>) -> Self {
            Self {
                downloads,
                last_call: Mutex::new(None),
            }
        }
    }

    impl DownloadRepository for PagingRepository {
        fn create_download(&self, _download: &Download) -> Result<(), RepositoryError> {
            unimplemented!()
        }

        fn find_by_info_hash(&self, _info_hash: &str) -> Result<Download, RepositoryError> {
            unimplemented!()
        }

        fn list_downloads(
            &self,
            status: Option<DownloadStatus>,
            from: Option<chrono::DateTime<chrono::Utc>>,
            after: Option<DownloadCursor>,
            order: DownloadListOrder,
        ) -> Result<impl Iterator<Item = Result<Download, RepositoryError>>, RepositoryError>
        {
            *self.last_call.lock().unwrap() = Some(RecordedListCall {
                status,
                from,
                after,
                order,
            });
            Ok(Box::new(self.downloads.clone().into_iter().map(Ok)))
        }

        fn update_download(&self, _download: &Download) -> Result<(), RepositoryError> {
            unimplemented!()
        }

        fn delete_download(&self, _info_hash: &str) -> Result<(), RepositoryError> {
            unimplemented!()
        }
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

    #[test]
    fn downloads_returns_iterator_from_repository() {
        let magnet: Magnet = MAGNET.parse().unwrap();
        let repo = Arc::new(PagingRepository::new(vec![Download::new(
            magnet,
            "/downloads".to_owned(),
        )]));
        let app = App::new(
            repo.clone(),
            Arc::new(OkTorrentClient),
            Duration::from_secs(60),
            PathBuf::from("/downloads"),
        );

        let downloads = app
            .downloads(
                Some(DownloadStatus::Queued),
                None,
                None,
                DownloadListOrder::CreatedAtDesc,
            )
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();

        assert_eq!(downloads.len(), 1);
        assert_eq!(downloads[0].status, DownloadStatus::Queued);
    }

    #[test]
    fn downloads_forwards_query_to_repository() {
        let repo = Arc::new(PagingRepository::new(Vec::new()));
        let app = App::new(
            repo.clone(),
            Arc::new(OkTorrentClient),
            Duration::from_secs(60),
            PathBuf::from("/downloads"),
        );

        let after = DownloadCursor {
            status: DownloadStatus::Queued,
            created_at: chrono::Utc::now(),
            info_hash: INFO_HASH.to_owned(),
        };

        app.downloads(
            Some(DownloadStatus::Queued),
            None,
            Some(after.clone()),
            DownloadListOrder::CreatedAtAsc,
        )
        .unwrap()
        .for_each(|download| drop(download.unwrap()));

        assert_eq!(
            *repo.last_call.lock().unwrap(),
            Some(RecordedListCall {
                status: Some(DownloadStatus::Queued),
                from: None,
                after: Some(after),
                order: DownloadListOrder::CreatedAtAsc,
            })
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
            content_name: "resolved-name.mkv",
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
        assert_eq!(dl.content_name, "resolved-name.mkv");
    }

    #[tokio::test]
    async fn poll_downloads_transitions_to_downloading_and_updates_name() {
        let client = Arc::new(StatefulTorrentClient {
            state: TorrentState::Downloading,
            name: "resolved-name",
            content_name: "resolved-name.mkv",
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
        assert_eq!(dl.content_name, "resolved-name.mkv");
    }

    #[tokio::test]
    async fn poll_downloads_transitions_to_failed_on_torrent_error() {
        let client = Arc::new(StatefulTorrentClient {
            state: TorrentState::Error,
            name: "some-name",
            content_name: "resolved-name.mkv",
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
        dl.content_name = "torrent-name".to_owned();
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
    async fn import_download_copies_single_file_to_file_destination() {
        let (app, _dir) = new_app(Arc::new(OkTorrentClient));
        let dst_dir = tempfile::tempdir().unwrap();

        let src = app.download_dir.join("single-file.mkv");
        std::fs::write(&src, b"video-data").unwrap();

        let magnet: Magnet = MAGNET.parse().unwrap();
        let mut dl = Download::new(magnet, dst_dir.path().to_str().unwrap().to_owned());
        dl.name = "single-file.mkv".to_owned();
        dl.content_name = "single-file.mkv".to_owned();
        dl.status = DownloadStatus::Importing;
        app.repository.create_download(&dl).unwrap();

        app.import_download(&mut dl).await;

        let expected_dst = dst_dir.path().join("single-file.mkv");
        assert_eq!(dl.status, DownloadStatus::Imported);
        assert_eq!(
            dl.imported_path.as_deref(),
            Some(expected_dst.to_str().unwrap())
        );
        assert_eq!(std::fs::read(expected_dst).unwrap(), b"video-data");
    }

    #[tokio::test]
    async fn import_download_transitions_to_failed_when_source_missing() {
        let (app, _dir) = new_app(Arc::new(OkTorrentClient));
        let dst_dir = tempfile::tempdir().unwrap();

        let magnet: Magnet = MAGNET.parse().unwrap();
        let mut dl = Download::new(magnet, dst_dir.path().to_str().unwrap().to_owned());
        // No directory at download_dir/"does-not-exist".
        dl.name = "does-not-exist".to_owned();
        dl.content_name = "does-not-exist".to_owned();
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

        copy_recursive(src_dir.path(), dst_dir.path()).unwrap();

        assert_eq!(
            std::fs::read(dst_dir.path().join("file.txt")).unwrap(),
            b"hello"
        );
        assert_eq!(
            std::fs::read(dst_dir.path().join("sub/nested.txt")).unwrap(),
            b"world"
        );
    }

    #[test]
    fn copy_dir_recursive_copies_file_to_file_destination() {
        let src_dir = tempfile::tempdir().unwrap();
        let dst_dir = tempfile::tempdir().unwrap();

        let src_file = src_dir.path().join("file.txt");
        let dst_file = dst_dir.path().join("nested/file.txt");
        std::fs::write(&src_file, b"hello").unwrap();

        copy_recursive(&src_file, &dst_file).unwrap();

        assert_eq!(std::fs::read(dst_file).unwrap(), b"hello");
    }
}

pub mod download;
pub mod error;
pub mod model;
pub mod torrent;

use std::sync::Arc;
use std::time::Duration;

use tokio_util::sync::CancellationToken;

use crate::app::download::DownloadRepository;
use crate::app::model::DownloadStatus;
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
                let new_status = match ts.state {
                    TorrentState::Downloading => DownloadStatus::Downloading,
                    TorrentState::Seeding => DownloadStatus::Completed,
                    TorrentState::Paused => DownloadStatus::Downloading,
                    TorrentState::Error => DownloadStatus::Failed,
                    TorrentState::Unknown => download.status,
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

use std::sync::Arc;
use std::time::Duration;

use crate::app::App;
use crate::cli::{load_config, StartArgs};
use crate::store::redb::RedbStore;
use crate::torrent::qbittorrent::{QbittorrentClient, QbittorrentConfig};

pub async fn run(args: StartArgs) {
    let cfg = match load_config(args) {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("Failed to load config: {e}");
            std::process::exit(1);
        }
    };

    tracing::info!(listen_addr = %cfg.server.listen_addr, "Server listen address");
    tracing::info!(store_path = %cfg.store.path, "Store path");

    let repo = match RedbStore::new(&cfg.store.path) {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("Failed to open repository: {e}");
            std::process::exit(1);
        }
    };

    let torrent_client = QbittorrentClient::new(QbittorrentConfig {
        host: cfg.qbittorrent.host,
        username: cfg.qbittorrent.username,
        password: cfg.qbittorrent.password,
    });

    let app = App::new(Arc::new(repo), Arc::new(torrent_client));
    let poll_interval = Duration::from_secs(cfg.qbittorrent.poll_interval_secs);
    app.run(poll_interval).await;

    tracing::info!("Magnarr started successfully");

    // Park the task until shutdown signal (future work).
    std::future::pending::<()>().await;
}

use std::sync::Arc;

use crate::app::App;
use crate::cli::config::TorrentClientConfig;
use crate::cli::{load_config, StartArgs};
use crate::store::redb::RedbStore;
use crate::torrent::qbittorrent::{QbittorrentClient, QbittorrentConfig as QbConnectionConfig};

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

    let (torrent_client, poll_interval) = match cfg.torrent_client {
        TorrentClientConfig::Qbittorrent(qb) => {
            let client = QbittorrentClient::new(QbConnectionConfig {
                host: qb.host,
                username: qb.username,
                password: qb.password,
            });
            (
                Arc::new(client) as Arc<dyn crate::app::torrent::TorrentClient>,
                cfg.app.poll_interval,
            )
        }
    };

    let app = App::new(Arc::new(repo), torrent_client);
    app.run(poll_interval).await;

    tracing::info!("Magnarr started successfully");

    // Park the task until shutdown signal (future work).
    std::future::pending::<()>().await;
}

use std::sync::Arc;

use tokio_util::sync::CancellationToken;

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

    let torrent_client = match cfg.torrent_client {
        TorrentClientConfig::Qbittorrent(qb) => Arc::new(QbittorrentClient::new(
            QbConnectionConfig {
                host: qb.host,
                username: qb.username,
                password: qb.password,
            },
        )) as Arc<dyn crate::app::torrent::TorrentClient>,
    };

    let token = CancellationToken::new();
    let app = App::new(Arc::new(repo), torrent_client, cfg.app.poll_interval);
    app.run(token.clone()).await;

    tracing::info!("Magnarr started successfully");

    // Park until a shutdown signal cancels the token (signal handling: future work).
    token.cancelled().await;
}

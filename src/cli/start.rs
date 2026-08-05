use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

use crate::app::App;
use crate::cli::config::{load_config, TorrentClientConfig};
use crate::cli::PathArg;
use crate::client::qbittorrent::{QbittorrentClient, QbittorrentConfig as QbConnectionConfig};
use crate::graphql::GraphqlServer;
use crate::store::DownloadStore;

#[derive(Default, Debug, serde::Serialize, clap::Parser)]
pub struct StartArgs {
    /// How often Magnarr polls the torrent client for download updates.
    #[arg(long, value_parser = humantime::parse_duration)]
    pub poll_interval: Option<Duration>,
    /// The directory where completed downloads are read from for imports.
    #[arg(long)]
    pub download_dir: Option<PathArg>,
    /// The HTTP listen address of the GraphQL server.
    #[arg(long)]
    pub listen_addr: Option<String>,
    /// The maximum page size accepted by the GraphQL API.
    #[arg(long)]
    pub max_page_size: Option<usize>,
    /// The path to the store file.
    #[arg(long)]
    pub store_path: Option<PathArg>,
    /// The qBittorrent base URL.
    #[arg(long)]
    pub qb_host: Option<String>,
    /// The qBittorrent username.
    #[arg(long)]
    pub qb_username: Option<String>,
    /// The qBittorrent password.
    #[arg(long)]
    pub qb_password: Option<String>,
}

pub async fn run(home: &Path, args: StartArgs) {
    let cfg = match load_config(home, args) {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("Failed to load config: {e}");
            std::process::exit(1);
        }
    };

    tracing::info!(listen_addr = %cfg.server.listen_addr, "Server listen address");

    let store_path = cfg.store.resolve_path(home);
    tracing::info!(store_path = %store_path.display(), "Store path");

    let repo = match DownloadStore::new(store_path) {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("Failed to open repository: {e}");
            std::process::exit(1);
        }
    };

    let torrent_client = match cfg.torrent_client {
        TorrentClientConfig::Qbittorrent(qb) => {
            Arc::new(QbittorrentClient::new(QbConnectionConfig {
                host: qb.host,
                username: qb.username,
                password: qb.password,
            }))
        }
    };

    let token = CancellationToken::new();
    let app = App::new(
        Arc::new(repo),
        torrent_client,
        cfg.app.poll_interval,
        cfg.app.resolve_download_dir(home),
    );
    app.start(token.clone());

    let graphql = GraphqlServer::new(Arc::new(app), cfg.server.max_page_size);
    let router = graphql.axum_router();

    let listener = match TcpListener::bind(&cfg.server.listen_addr).await {
        Ok(l) => l,
        Err(e) => {
            tracing::error!(addr = %cfg.server.listen_addr, "Failed to bind server: {e}");
            std::process::exit(1);
        }
    };

    tracing::info!(addr = %cfg.server.listen_addr, "GraphQL server listening");

    let shutdown = token.clone().cancelled_owned();
    tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, router)
            .with_graceful_shutdown(shutdown)
            .await
        {
            tracing::error!("HTTP server error: {e}");
        }
    });

    tracing::info!("Magnarr started successfully");

    token.cancelled().await;
}

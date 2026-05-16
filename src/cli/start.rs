use std::path::PathBuf;
use std::sync::Arc;
use clap::Parser;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

use crate::app::App;
use crate::cli::config::TorrentClientConfig;
use crate::cli::load_config;
use crate::client::qbittorrent::{QbittorrentClient, QbittorrentConfig as QbConnectionConfig};
use crate::graphql::GraphqlServer;
use crate::store::redb::RedbStore;

#[derive(Debug, Parser, serde::Serialize)]
pub struct StartArgs {
    /// Server listen address
    #[arg(long)]
    pub server_listen_addr: Option<String>,

    /// Store path
    #[arg(long)]
    pub store_path: Option<String>,
}

pub async fn run(home: &PathBuf, args: StartArgs) {
    let cfg = match load_config(home, args) {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("Failed to load config: {e}");
            std::process::exit(1);
        }
    };

    tracing::info!(listen_addr = %cfg.server.listen_addr, "Server listen address");
    tracing::info!(store_path = %cfg.store.path, "Store path");

    let repo = match RedbStore::new(cfg.store.resolve_path(home)) {
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
            })) as Arc<dyn crate::app::torrent::TorrentClient>
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

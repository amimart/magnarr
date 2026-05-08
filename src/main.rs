use clap::Parser;
use magnarr::config::{load_config, Cli};
use magnarr::store::redb::RedbStore;

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();
    let cfg = match load_config(cli) {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("Failed to load config: {e}");
            std::process::exit(1);
        }
    };

    tracing::info!(listen_addr = %cfg.server.listen_addr, "Server listen address");
    tracing::info!(store_path = %cfg.store.path, "Store path");

    if let Err(e) = RedbStore::new(&cfg.store.path) {
        tracing::error!("Failed to open store: {e}");
        std::process::exit(1);
    }

    tracing::info!("Magnarr started successfully");
}

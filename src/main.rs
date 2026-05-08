use clap::Parser;
use magnarr::config::{load_config, Cli, Command};
use magnarr::store::redb::RedbStore;

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();

    match cli.command {
        Command::Version => print_version(),
        Command::Start(args) => {
            let cfg = match load_config(args) {
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
    }
}

fn print_version() {
    let version = env!("CARGO_PKG_VERSION");
    let git_hash = env!("MAGNARR_GIT_HASH");
    let build_ts: i64 = env!("MAGNARR_BUILD_TIMESTAMP").parse().unwrap_or(0);
    let build_date = chrono::DateTime::from_timestamp(build_ts, 0)
        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S UTC").to_string())
        .unwrap_or_else(|| "unknown".to_owned());

    println!("magnarr {version}");
    println!("Commit:  {git_hash}");
    println!("Built:   {build_date}");
}

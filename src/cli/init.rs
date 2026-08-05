use crate::cli::config::{get_config_path, Config};
use crate::store::DownloadStore;
use std::path::Path;

pub fn run(home: &Path) {
    if home.exists() {
        tracing::error!("Home directory already exists at: {}", home.display());
        std::process::exit(1);
    }

    tracing::info!(home = %home.display(), "Initializing home directory...");

    std::fs::create_dir_all(home).unwrap_or_else(|e| {
        tracing::error!("Failed to create home directory: {e}");
        std::process::exit(1);
    });

    let cfg = Config::default();
    let yaml = serde_yaml::to_string(&cfg).unwrap_or_else(|e| {
        tracing::error!("Failed to serialize default config: {e}");
        std::process::exit(1);
    });

    let config_path = get_config_path(home);
    std::fs::write(&config_path, yaml).unwrap_or_else(|e| {
        tracing::error!("Failed to write default config: {e}");
        std::process::exit(1);
    });
    tracing::info!(path = %config_path.display(), "Created config file with default settings");

    let store_path = cfg.store.resolve_path(home);
    DownloadStore::new(store_path.clone()).unwrap_or_else(|e| {
        tracing::error!("Failed to initialize store: {e}");
        std::process::exit(1);
    });
    tracing::info!(path = %store_path.display(), "Initialized store data");
}

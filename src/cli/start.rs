use crate::app::App;
use crate::cli::{load_config, StartArgs};
use crate::store::redb::RedbStore;

pub fn run(args: StartArgs) {
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

    let _app = App::new(Box::new(repo));
    tracing::info!("Magnarr started successfully");
}

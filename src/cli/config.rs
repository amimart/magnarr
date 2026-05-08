use std::path::Path;

use thiserror::Error;

use super::StartArgs;

#[derive(Debug, Clone, serde::Deserialize)]
pub struct Config {
    pub server: ServerConfig,
    pub store: StoreConfig,
    pub torrent_client: TorrentClientConfig,
}

/// Discriminated union of supported torrent clients.
/// The `kind` field selects the variant; remaining fields are client-specific.
///
/// Example config file:
/// ```yaml
/// torrent_client:
///   kind: qbittorrent
///   host: http://localhost:8080
///   username: admin
///   password: secret
/// ```
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TorrentClientConfig {
    Qbittorrent(QbittorrentConfig),
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct ServerConfig {
    pub listen_addr: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct StoreConfig {
    pub path: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct QbittorrentConfig {
    #[serde(default = "default_qb_host")]
    pub host: String,
    #[serde(default = "default_qb_username")]
    pub username: String,
    #[serde(default = "default_qb_password")]
    pub password: String,
    #[serde(default = "default_qb_poll_interval_secs")]
    pub poll_interval_secs: u64,
}

fn default_qb_host() -> String {
    "http://localhost:8080".to_owned()
}

fn default_qb_username() -> String {
    "admin".to_owned()
}

fn default_qb_password() -> String {
    "adminadmin".to_owned()
}

fn default_qb_poll_interval_secs() -> u64 {
    30
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("config file not found: {0}")]
    FileNotFound(String),
    #[error("failed to read config file: {0}")]
    Io(#[from] std::io::Error),
    #[error("failed to parse config file: {0}")]
    Parse(#[from] serde_yaml::Error),
}

/// Partial config overlaid from a config file (all fields optional for merging).
#[derive(Debug, Default, serde::Deserialize)]
struct FileConfig {
    server: Option<FileServerConfig>,
    store: Option<FileStoreConfig>,
    /// When present, replaces the entire torrent_client block.
    /// Fields within the variant use serde defaults, so partial specification is fine.
    torrent_client: Option<TorrentClientConfig>,
}

#[derive(Debug, serde::Deserialize)]
struct FileServerConfig {
    listen_addr: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
struct FileStoreConfig {
    path: Option<String>,
}

fn default_config() -> Config {
    Config {
        server: ServerConfig {
            listen_addr: "127.0.0.1:8080".to_owned(),
        },
        store: StoreConfig {
            path: "./data/magnarr.redb".to_owned(),
        },
        torrent_client: TorrentClientConfig::Qbittorrent(QbittorrentConfig {
            host: default_qb_host(),
            username: default_qb_username(),
            password: default_qb_password(),
            poll_interval_secs: default_qb_poll_interval_secs(),
        }),
    }
}

/// Load config respecting precedence: defaults < file < env vars < CLI args.
pub fn load_config(args: StartArgs) -> Result<Config, ConfigError> {
    let mut cfg = default_config();

    // --- config file ---
    let config_path = args.config.as_str();
    let is_default_path = config_path == "config.yaml";

    if Path::new(config_path).exists() {
        let contents = std::fs::read_to_string(config_path)?;
        let file_cfg: FileConfig = serde_yaml::from_str(&contents)?;

        if let Some(server) = file_cfg.server {
            if let Some(addr) = server.listen_addr {
                cfg.server.listen_addr = addr;
            }
        }
        if let Some(store) = file_cfg.store {
            if let Some(path) = store.path {
                cfg.store.path = path;
            }
        }
        if let Some(tc) = file_cfg.torrent_client {
            cfg.torrent_client = tc;
        }
    } else if !is_default_path {
        return Err(ConfigError::FileNotFound(config_path.to_owned()));
    }

    // --- env vars ---
    if let Ok(addr) = std::env::var("MAGNARR_SERVER_LISTEN_ADDR") {
        cfg.server.listen_addr = addr;
    }
    if let Ok(path) = std::env::var("MAGNARR_STORE_PATH") {
        cfg.store.path = path;
    }
    match cfg.torrent_client {
        TorrentClientConfig::Qbittorrent(ref mut qb) => {
            if let Ok(host) = std::env::var("MAGNARR_QB_HOST") {
                qb.host = host;
            }
            if let Ok(username) = std::env::var("MAGNARR_QB_USERNAME") {
                qb.username = username;
            }
            if let Ok(password) = std::env::var("MAGNARR_QB_PASSWORD") {
                qb.password = password;
            }
            if let Ok(secs) = std::env::var("MAGNARR_QB_POLL_INTERVAL_SECS") {
                if let Ok(v) = secs.parse() {
                    qb.poll_interval_secs = v;
                }
            }
        }
    }

    // --- CLI args ---
    if let Some(addr) = args.server_listen_addr {
        cfg.server.listen_addr = addr;
    }
    if let Some(path) = args.store_path {
        cfg.store.path = path;
    }

    Ok(cfg)
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::sync::Mutex;

    use super::*;
    use crate::cli::StartArgs;

    /// Serializes tests that modify environment variables to avoid cross-test pollution.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn default_start_args() -> StartArgs {
        StartArgs {
            config: "config.yaml".to_owned(),
            server_listen_addr: None,
            store_path: None,
        }
    }

    #[test]
    fn default_config_values_are_correct() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("MAGNARR_SERVER_LISTEN_ADDR");
        std::env::remove_var("MAGNARR_STORE_PATH");
        let cfg = load_config(default_start_args()).unwrap();
        assert_eq!(cfg.server.listen_addr, "127.0.0.1:8080");
        assert_eq!(cfg.store.path, "./data/magnarr.redb");
    }

    #[test]
    fn config_file_loading_applies_file_values() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("MAGNARR_SERVER_LISTEN_ADDR");
        std::env::remove_var("MAGNARR_STORE_PATH");

        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.yaml");
        let mut f = std::fs::File::create(&config_path).unwrap();
        writeln!(
            f,
            "server:\n  listen_addr: 0.0.0.0:9090\nstore:\n  path: /tmp/test.redb"
        )
        .unwrap();

        let args = StartArgs {
            config: config_path.to_str().unwrap().to_owned(),
            server_listen_addr: None,
            store_path: None,
        };
        let cfg = load_config(args).unwrap();
        assert_eq!(cfg.server.listen_addr, "0.0.0.0:9090");
        assert_eq!(cfg.store.path, "/tmp/test.redb");
    }

    #[test]
    fn env_var_listen_addr_overrides_config_file() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("MAGNARR_STORE_PATH");

        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.yaml");
        let mut f = std::fs::File::create(&config_path).unwrap();
        writeln!(f, "server:\n  listen_addr: 0.0.0.0:9090").unwrap();

        std::env::set_var("MAGNARR_SERVER_LISTEN_ADDR", "1.2.3.4:1111");
        let args = StartArgs {
            config: config_path.to_str().unwrap().to_owned(),
            server_listen_addr: None,
            store_path: None,
        };
        let cfg = load_config(args).unwrap();
        std::env::remove_var("MAGNARR_SERVER_LISTEN_ADDR");
        assert_eq!(cfg.server.listen_addr, "1.2.3.4:1111");
    }

    #[test]
    fn env_var_store_path_overrides_config_file() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("MAGNARR_SERVER_LISTEN_ADDR");

        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.yaml");
        let mut f = std::fs::File::create(&config_path).unwrap();
        writeln!(f, "store:\n  path: /original.redb").unwrap();

        std::env::set_var("MAGNARR_STORE_PATH", "/env-override.redb");
        let args = StartArgs {
            config: config_path.to_str().unwrap().to_owned(),
            server_listen_addr: None,
            store_path: None,
        };
        let cfg = load_config(args).unwrap();
        std::env::remove_var("MAGNARR_STORE_PATH");
        assert_eq!(cfg.store.path, "/env-override.redb");
    }

    #[test]
    fn cli_arg_overrides_everything() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("MAGNARR_STORE_PATH");

        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.yaml");
        let mut f = std::fs::File::create(&config_path).unwrap();
        writeln!(f, "server:\n  listen_addr: 0.0.0.0:9090").unwrap();

        std::env::set_var("MAGNARR_SERVER_LISTEN_ADDR", "1.2.3.4:1111");
        let args = StartArgs {
            config: config_path.to_str().unwrap().to_owned(),
            server_listen_addr: Some("5.6.7.8:2222".to_owned()),
            store_path: None,
        };
        let cfg = load_config(args).unwrap();
        std::env::remove_var("MAGNARR_SERVER_LISTEN_ADDR");
        assert_eq!(cfg.server.listen_addr, "5.6.7.8:2222");
    }

    #[test]
    fn explicit_config_path_not_found_returns_error() {
        let args = StartArgs {
            config: "/nonexistent/path/config.yaml".to_owned(),
            server_listen_addr: None,
            store_path: None,
        };
        let result = load_config(args);
        assert!(matches!(result, Err(ConfigError::FileNotFound(_))));
    }

    #[test]
    fn missing_default_config_file_is_silently_ignored() {
        let args = StartArgs {
            config: "config.yaml".to_owned(),
            server_listen_addr: None,
            store_path: None,
        };
        if !Path::new("config.yaml").exists() {
            assert!(load_config(args).is_ok());
        }
    }

    #[test]
    fn default_torrent_client_is_qbittorrent_with_correct_defaults() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("MAGNARR_QB_HOST");
        std::env::remove_var("MAGNARR_QB_USERNAME");
        std::env::remove_var("MAGNARR_QB_PASSWORD");
        std::env::remove_var("MAGNARR_QB_POLL_INTERVAL_SECS");

        let cfg = load_config(default_start_args()).unwrap();
        let TorrentClientConfig::Qbittorrent(qb) = cfg.torrent_client;
        assert_eq!(qb.host, "http://localhost:8080");
        assert_eq!(qb.username, "admin");
        assert_eq!(qb.poll_interval_secs, 30);
    }

    #[test]
    fn config_file_torrent_client_section_is_loaded() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("MAGNARR_QB_HOST");
        std::env::remove_var("MAGNARR_QB_USERNAME");
        std::env::remove_var("MAGNARR_QB_PASSWORD");
        std::env::remove_var("MAGNARR_QB_POLL_INTERVAL_SECS");

        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.yaml");
        let mut f = std::fs::File::create(&config_path).unwrap();
        writeln!(
            f,
            "torrent_client:\n  kind: qbittorrent\n  host: http://myserver:9090\n  poll_interval_secs: 60"
        )
        .unwrap();

        let args = StartArgs {
            config: config_path.to_str().unwrap().to_owned(),
            server_listen_addr: None,
            store_path: None,
        };
        let cfg = load_config(args).unwrap();
        let TorrentClientConfig::Qbittorrent(qb) = cfg.torrent_client;
        assert_eq!(qb.host, "http://myserver:9090");
        assert_eq!(qb.poll_interval_secs, 60);
        // Fields not specified in the file fall back to per-field serde defaults.
        assert_eq!(qb.username, "admin");
    }

    #[test]
    fn env_vars_override_qbittorrent_fields() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("MAGNARR_QB_HOST", "http://envhost:7777");
        std::env::set_var("MAGNARR_QB_POLL_INTERVAL_SECS", "120");
        std::env::remove_var("MAGNARR_QB_USERNAME");
        std::env::remove_var("MAGNARR_QB_PASSWORD");

        let cfg = load_config(default_start_args()).unwrap();
        std::env::remove_var("MAGNARR_QB_HOST");
        std::env::remove_var("MAGNARR_QB_POLL_INTERVAL_SECS");

        let TorrentClientConfig::Qbittorrent(qb) = cfg.torrent_client;
        assert_eq!(qb.host, "http://envhost:7777");
        assert_eq!(qb.poll_interval_secs, 120);
    }
}

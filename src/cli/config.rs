use std::path::{Path, PathBuf};
use std::time::Duration;

use super::{PathArg, StartArgs};

#[derive(Default, Debug, Clone, serde::Deserialize)]
#[serde(default)]
pub struct Config {
    pub app: AppConfig,
    pub server: ServerConfig,
    pub store: StoreConfig,
    pub torrent_client: TorrentClientConfig,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(default)]
pub struct AppConfig {
    /// How often the app polls the torrent client for status updates.
    /// Accepts humantime strings: "30s", "1m", "5 minutes", etc.
    #[serde(with = "humantime_serde")]
    pub poll_interval: Duration,
    /// Directory where the torrent client saves completed downloads.
    pub download_dir: PathArg,
}

impl AppConfig {
    pub fn resolve_download_dir(&self, home: &Path) -> PathBuf {
        home_relative_or_absolute_path(home, &self.download_dir)
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            poll_interval: Duration::from_secs(30),
            download_dir: PathArg(PathBuf::from("./download")),
        }
    }
}

/// Discriminated union of supported torrent clients.
///
/// Example config file:
/// ```yaml
/// torrent_client:
///   qbittorrent:
///     host: http://localhost:9393
///     username: admin
///     password: secret
/// ```
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TorrentClientConfig {
    Qbittorrent(QbittorrentConfig),
}

impl Default for TorrentClientConfig {
    fn default() -> Self {
        Self::Qbittorrent(QbittorrentConfig::default())
    }
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(default)]
pub struct ServerConfig {
    pub listen_addr: String,
    pub max_page_size: usize,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            listen_addr: "http://localhost:9393".to_string(),
            max_page_size: 100,
        }
    }
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(default)]
pub struct StoreConfig {
    pub path: PathArg,
}

impl StoreConfig {
    pub fn resolve_path(&self, home: &Path) -> PathBuf {
        home_relative_or_absolute_path(home, &self.path.0)
    }
}

impl Default for StoreConfig {
    fn default() -> Self {
        Self {
            path: PathArg(PathBuf::from("./data/magnarr.redb")),
        }
    }
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(default)]
pub struct QbittorrentConfig {
    pub host: String,
    pub username: String,
    pub password: String,
}

impl Default for QbittorrentConfig {
    fn default() -> Self {
        Self {
            host: "http://localhost:8080".to_string(),
            username: "admin".to_string(),
            password: "adminadmin".to_string(),
        }
    }
}

/// Load config respecting precedence: defaults < file < env vars < CLI args.
pub fn load_config(home: &Path, args: StartArgs) -> Result<Config, config::ConfigError> {
    config::Config::builder()
        .add_source(config::File::from(home.join("config.yaml")).required(false))
        .add_source(config::Environment::with_prefix("MAGNARR").separator("_"))
        .set_override_option("app.poll_interval", args.poll_interval.map(|d| humantime::format_duration(d).to_string()))?
        .set_override_option("app.download_dir", args.download_dir.map(|d| d.to_str().map(|s| s.to_owned())).flatten())?
        .set_override_option("server.listen_addr", args.listen_addr)?
        .set_override_option("server.max_page_size", args.max_page_size.map(|m| m as u64))?
        .set_override_option("store.path", args.store_path.map(|d| d.to_str().map(|s| s.to_owned())).flatten())?
        .set_override_option("torrent_client.qbittorrent.host", args.qb_host)?
        .set_override_option("torrent_client.qbittorrent.username", args.qb_username)?
        .set_override_option("torrent_client.qbittorrent.password", args.qb_password)?
        .build()?
        .try_deserialize::<Config>()
}

pub fn home_relative_or_absolute_path(home: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        home.join(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};

    fn env_lock() -> &'static Mutex<()> {
        static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        ENV_LOCK.get_or_init(|| Mutex::new(()))
    }

    fn empty_args() -> StartArgs {
        StartArgs {
            poll_interval: None,
            download_dir: None,
            listen_addr: None,
            max_page_size: None,
            store_path: None,
            qb_host: None,
            qb_username: None,
            qb_password: None,
        }
    }

    fn write_config(home: &Path, content: &str) {
        std::fs::create_dir_all(home).unwrap();
        std::fs::write(home.join("config.yaml"), content).unwrap();
    }

    fn clear_test_env() {
        for key in [
            "MAGNARR_APP_POLL_INTERVAL",
            "MAGNARR_APP_DOWNLOAD_DIR",
            "MAGNARR_SERVER_LISTEN_ADDR",
            "MAGNARR_SERVER_MAX_PAGE_SIZE",
            "MAGNARR_STORE_PATH",
            "MAGNARR_TORRENT_CLIENT_HOST",
            "MAGNARR_TORRENT_CLIENT_USERNAME",
            "MAGNARR_TORRENT_CLIENT_USERNAME",
            "MAGNARR_TORRENT_CLIENT_PASSWORD",
        ] {
            unsafe { std::env::remove_var(key) };
        }
    }

    #[test]
    fn load_config_uses_defaults_without_other_sources() {
        let _env_lock = env_lock().lock().unwrap_or_else(|e| e.into_inner());
        clear_test_env();

        let home = tempfile::tempdir().unwrap();
        let cfg = load_config(&home.path().to_path_buf(), empty_args()).unwrap();

        assert_eq!(cfg.app.poll_interval, Duration::from_secs(30));
        assert_eq!(cfg.app.download_dir.0, PathBuf::from("./download"));
        assert_eq!(cfg.server.listen_addr, "http://localhost:9393");
        assert_eq!(cfg.server.max_page_size, 100);
        assert_eq!(cfg.store.path.0, PathBuf::from("./data/magnarr.redb"));
        match cfg.torrent_client {
            TorrentClientConfig::Qbittorrent(qb) => {
                assert_eq!(qb.host, "http://localhost:8080");
                assert_eq!(qb.username, "admin");
                assert_eq!(qb.password, "adminadmin");
            }
        }
    }

    #[test]
    fn load_config_reads_config_file_values() {
        let _env_lock = env_lock().lock().unwrap_or_else(|e| e.into_inner());
        clear_test_env();

        let home = tempfile::tempdir().unwrap();
        write_config(
            &home.path().to_path_buf(),
            r#"
app:
  poll_interval: 45s
  download_dir: ./downloads-from-file
server:
  listen_addr: 127.0.0.1:4000
  max_page_size: 75
store:
  path: ./store-from-file.redb
torrent_client:
  qbittorrent:
    host: http://qb-from-file:8081
    username: file-user
    password: file-pass
"#,
        );

        let cfg = load_config(&home.path().to_path_buf(), empty_args()).unwrap();

        assert_eq!(cfg.app.poll_interval, Duration::from_secs(45));
        assert_eq!(
            cfg.app.download_dir.0,
            PathBuf::from("./downloads-from-file")
        );
        assert_eq!(cfg.server.listen_addr, "127.0.0.1:4000");
        assert_eq!(cfg.server.max_page_size, 75);
        assert_eq!(cfg.store.path.0, PathBuf::from("./store-from-file.redb"));
        match cfg.torrent_client {
            TorrentClientConfig::Qbittorrent(qb) => {
                assert_eq!(qb.host, "http://qb-from-file:8081");
                assert_eq!(qb.username, "file-user");
                assert_eq!(qb.password, "file-pass");
            }
        }
    }

    #[test]
    fn load_config_applies_override_chain_defaults_file_env_cli() {
        let _env_lock = env_lock().lock().unwrap_or_else(|e| e.into_inner());
        clear_test_env();

        let home = tempfile::tempdir().unwrap();
        write_config(
            &home.path().to_path_buf(),
            r#"
app:
  poll_interval: 45s
  download_dir: ./downloads-from-file
server:
  listen_addr: 127.0.0.1:4000
  max_page_size: 75
store:
  path: ./store-from-file.redb
torrent_client:
  qbittorrent:
    host: http://qb-from-file:8081
    username: file-user
    password: file-pass
"#,
        );

        unsafe {
            std::env::set_var("MAGNARR_APP_POLL_INTERVAL", "1m");
            std::env::set_var("MAGNARR_APP_DOWNLOAD_DIR", "./downloads-from-env");
            std::env::set_var("MAGNARR_SERVER_LISTEN_ADDR", "0.0.0.0:5000");
            std::env::set_var("MAGNARR_SERVER_MAX_PAGE_SIZE", "80");
            std::env::set_var("MAGNARR_STORE_PATH", "./store-from-env.redb");
        }

        let cfg = load_config(
            &home.path().to_path_buf(),
            StartArgs {
                poll_interval: Some(Duration::from_secs(90)),
                download_dir: Some(PathArg(PathBuf::from("./downloads-from-cli"))),
                listen_addr: Some("0.0.0.0:6000".to_owned()),
                max_page_size: Some(90),
                store_path: Some(PathArg(PathBuf::from("./store-from-cli.redb"))),
                qb_host: Some("http://qb-from-cli:8082".to_owned()),
                qb_username: Some("cli-user".to_owned()),
                qb_password: Some("cli-pass".to_owned()),
            },
        )
        .unwrap();

        clear_test_env();

        assert_eq!(cfg.app.poll_interval, Duration::from_secs(90));
        assert_eq!(
            cfg.app.download_dir.0,
            PathBuf::from("./downloads-from-cli")
        );
        assert_eq!(cfg.server.listen_addr, "0.0.0.0:6000");
        assert_eq!(cfg.server.max_page_size, 90);
        assert_eq!(cfg.store.path.0, PathBuf::from("./store-from-cli.redb"));
        match cfg.torrent_client {
            TorrentClientConfig::Qbittorrent(qb) => {
                assert_eq!(qb.host, "http://qb-from-cli:8082");
                assert_eq!(qb.username, "cli-user");
                assert_eq!(qb.password, "cli-pass");
            }
        }
    }
}

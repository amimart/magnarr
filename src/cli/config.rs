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

use std::path::PathBuf;
use std::time::Duration;

use super::{PathArg, StartArgs};

#[derive(Debug, Clone, serde::Deserialize)]
pub struct Config {
    pub app: AppConfig,
    pub server: ServerConfig,
    pub store: StoreConfig,
    pub torrent_client: TorrentClientConfig,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct AppConfig {
    /// How often the app polls the torrent client for status updates.
    /// Accepts humantime strings: "30s", "1m", "5 minutes", etc.
    #[serde(with = "humantime_serde", default = "default_poll_interval")]
    pub poll_interval: Duration,
    /// Directory where the torrent client saves completed downloads.
    #[serde(default = "default_download_dir")]
    pub download_dir: PathArg,
}

impl AppConfig {
    pub fn resolve_download_dir(&self, home: &PathBuf) -> PathBuf {
        home_relative_or_absolute_path(home, &self.download_dir)
    }
}

fn default_poll_interval() -> Duration {
    Duration::from_secs(30)
}

fn default_download_dir() -> PathArg {
    PathArg(PathBuf::from("./download"))
}

/// Discriminated union of supported torrent clients.
/// The `kind` field selects the variant; remaining fields are client-specific.
///
/// Example config file:
/// ```yaml
/// torrent_client:
///   kind: qbittorrent
///   host: http://localhost:9393
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
    #[serde(default = "default_server_listen_addr")]
    pub listen_addr: String,
    #[serde(default = "default_server_max_page_size")]
    pub max_page_size: usize,
}

fn default_server_listen_addr() -> String {
    "http://localhost:9393".to_string()
}

fn default_server_max_page_size() -> usize {
    100
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct StoreConfig {
    #[serde(default = "default_store_path")]
    pub path: PathArg,
}

impl StoreConfig {
    pub fn resolve_path(&self, home: &PathBuf) -> PathBuf {
        home_relative_or_absolute_path(home, &self.path.0)
    }
}

fn default_store_path() -> PathArg {
    PathArg(PathBuf::from("./data/magnarr.redb"))
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct QbittorrentConfig {
    #[serde(default = "default_qb_host")]
    pub host: String,
    #[serde(default = "default_qb_username")]
    pub username: String,
    #[serde(default = "default_qb_password")]
    pub password: String,
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

/// Load config respecting precedence: defaults < file < env vars < CLI args.
pub fn load_config(home: &PathBuf, args: StartArgs) -> Result<Config, config::ConfigError> {
    config::Config::builder()
        .add_source(config::File::from(home.join("config.yaml")))
        .add_source(config::Environment::with_prefix("MAGNARR").separator("_"))
        .add_source(config::Config::try_from(&args)?)
        .build()?
        .try_deserialize::<Config>()
}

pub fn home_relative_or_absolute_path(home: &PathBuf, path: &PathBuf) -> PathBuf {
    if path.is_absolute() {
        path.clone()
    } else {
        home.join(&path)
    }
}

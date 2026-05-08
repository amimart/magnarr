use async_trait::async_trait;
use thiserror::Error;

use crate::app::model::MagnetUri;

#[derive(Debug, Error)]
pub enum TorrentClientError {
    #[error("authentication failed")]
    AuthFailed,
    #[error("torrent not found: {0}")]
    NotFound(String),
    #[error("API error: {0}")]
    Api(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TorrentState {
    Downloading,
    Seeding,
    Paused,
    Error,
    Unknown,
}

#[derive(Debug, Clone)]
pub struct TorrentStatus {
    pub state: TorrentState,
    /// Download progress from 0.0 to 1.0.
    pub progress: f32,
    /// Estimated seconds remaining, if known.
    pub eta: Option<u64>,
    pub download_speed: u64,
    pub upload_speed: u64,
    pub peers: u32,
    pub save_path: String,
}

#[async_trait]
pub trait TorrentClient: Send + Sync {
    async fn download(&self, magnet: &MagnetUri) -> Result<(), TorrentClientError>;
    async fn status(&self, info_hash: &str) -> Result<TorrentStatus, TorrentClientError>;
}

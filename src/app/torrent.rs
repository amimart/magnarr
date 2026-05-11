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
    /// Hash of the torrent.
    pub hash: String,
    /// Current state of the torrent.
    pub state: TorrentState,
    /// Torrent name.
    pub name: String,
}

#[async_trait]
pub trait TorrentClient: Send + Sync {
    async fn download(&self, magnet: &MagnetUri) -> Result<(), TorrentClientError>;
    async fn status(&self, info_hash: &str) -> Result<TorrentStatus, TorrentClientError>;
}

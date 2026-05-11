use async_trait::async_trait;
use thiserror::Error;

use crate::types::{MagnetUri, TorrentStatus};

#[derive(Debug, Error)]
pub enum TorrentClientError {
    #[error("authentication failed")]
    AuthFailed,
    #[error("torrent not found: {0}")]
    NotFound(String),
    #[error("API error: {0}")]
    Api(String),
}

#[async_trait]
pub trait TorrentClient: Send + Sync {
    async fn download(&self, magnet: &MagnetUri) -> Result<(), TorrentClientError>;
    async fn status(&self, info_hash: &str) -> Result<TorrentStatus, TorrentClientError>;
}

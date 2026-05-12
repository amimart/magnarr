use async_trait::async_trait;
use reqwest::StatusCode;
use thiserror::Error;

use crate::types::{Magnet, TorrentStatus};

#[derive(Debug, Error)]
pub enum TorrentClientError {
    #[error("authentication failed: {0}")]
    AuthFailed(StatusCode),
    #[error("torrent not found: {0}")]
    NotFound(String),
    #[error("Unexpected status code: {0}")]
    UnexpectedStatus(StatusCode),
    #[error(transparent)]
    ClientError(#[from] reqwest::Error),
}

#[async_trait]
pub trait TorrentClient: Send + Sync {
    async fn download(&self, magnet: &Magnet) -> Result<(), TorrentClientError>;
    async fn status(&self, info_hash: &str) -> Result<TorrentStatus, TorrentClientError>;
}

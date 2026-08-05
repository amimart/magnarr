use thiserror::Error;

use crate::app::repository::RepositoryError;
use crate::app::torrent::TorrentClientError;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("download already exists")]
    AlreadyExists,
    #[error("repository error: {0}")]
    Repository(#[from] RepositoryError),
    #[error("torrent client error: {0}")]
    TorrentClient(#[from] TorrentClientError),
}

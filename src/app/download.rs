use thiserror::Error;

use crate::types::{Download, DownloadStatus};

#[derive(Debug, Error)]
pub enum RepositoryError {
    #[error("not found")]
    NotFound,
    #[error("already exists")]
    AlreadyExists,
    #[error("backend error: {0}")]
    Backend(String),
    #[error("serialization error: {0}")]
    Serialization(String),
}

pub trait DownloadRepository: Send + Sync {
    fn create_download(&self, download: &Download) -> Result<(), RepositoryError>;
    fn find_by_info_hash(&self, info_hash: &str) -> Result<Download, RepositoryError>;
    fn list_downloads_by_status(
        &self,
        status: DownloadStatus,
    ) -> Result<Vec<Download>, RepositoryError>;
    fn update_download(&self, download: &Download) -> Result<(), RepositoryError>;
    fn delete_download(&self, info_hash: &str) -> Result<(), RepositoryError>;
}

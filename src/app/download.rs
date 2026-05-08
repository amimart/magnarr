use thiserror::Error;

use crate::app::model::Download;

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
    fn get_download(&self, id: uuid::Uuid) -> Result<Download, RepositoryError>;
    fn find_by_info_hash(&self, info_hash: &str) -> Result<Option<Download>, RepositoryError>;
    fn list_downloads(&self) -> Result<Vec<Download>, RepositoryError>;
    fn update_download(&self, download: &Download) -> Result<(), RepositoryError>;
    fn delete_download(&self, id: uuid::Uuid) -> Result<(), RepositoryError>;
}

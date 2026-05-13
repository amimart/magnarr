use thiserror::Error;

use crate::types::{Download, DownloadStatus};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortOrder {
    Asc,
    #[default]
    Desc,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DownloadCursor {
    pub status: DownloadStatus,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub info_hash: String,
}

impl DownloadCursor {
    pub fn from_download(download: &Download) -> Self {
        Self {
            status: download.status,
            created_at: download.created_at,
            info_hash: download.info_hash.clone(),
        }
    }
}

#[derive(Debug, Error)]
pub enum RepositoryError {
    #[error("not found")]
    NotFound,
    #[error("already exists")]
    AlreadyExists,
    #[error("backend error: {0}")]
    Storage(String),
    #[error("serialization error: {0}")]
    Serde(String),
}

pub trait DownloadRepository: Send + Sync {
    fn insert(&self, download: &Download) -> Result<(), RepositoryError>;
    fn get(&self, info_hash: &str) -> Result<Download, RepositoryError>;
    fn list(
        &self,
        status: Option<DownloadStatus>,
        from: Option<chrono::DateTime<chrono::Utc>>,
        after: Option<DownloadCursor>,
        order: SortOrder,
    ) -> Result<impl Iterator<Item = Result<Download, RepositoryError>>, RepositoryError>;
    fn update(&self, download: &Download) -> Result<(), RepositoryError>;
    fn remove(&self, info_hash: &str) -> Result<(), RepositoryError>;
}

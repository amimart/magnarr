use thiserror::Error;

use crate::types::{Download, DownloadStatus};

pub const DEFAULT_DOWNLOADS_PAGE_SIZE: usize = 50;
pub const MAX_DOWNLOADS_PAGE_SIZE: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DownloadListOrder {
    CreatedAtAsc,
    #[default]
    CreatedAtDesc,
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
    Backend(String),
    #[error("serialization error: {0}")]
    Serialization(String),
}

pub trait DownloadRepository: Send + Sync {
    fn create_download(&self, download: &Download) -> Result<(), RepositoryError>;
    fn find_by_info_hash(&self, info_hash: &str) -> Result<Download, RepositoryError>;
    fn list_downloads(
        &self,
        status: Option<DownloadStatus>,
        from: Option<chrono::DateTime<chrono::Utc>>,
        after: Option<DownloadCursor>,
        order: DownloadListOrder,
    ) -> Result<impl Iterator<Item=Result<Download, RepositoryError>>, RepositoryError>;
    fn update_download(&self, download: &Download) -> Result<(), RepositoryError>;
    fn delete_download(&self, info_hash: &str) -> Result<(), RepositoryError>;
}

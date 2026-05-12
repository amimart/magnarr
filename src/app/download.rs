use thiserror::Error;

use crate::types::{Download, DownloadStatus};

pub const DEFAULT_DOWNLOADS_PAGE_SIZE: usize = 50;
pub const MAX_DOWNLOADS_PAGE_SIZE: usize = 100;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadsPageCursor {
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub info_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadsPage {
    pub downloads: Vec<Download>,
    pub end_cursor: Option<DownloadsPageCursor>,
    pub has_next_page: bool,
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
    fn list_downloads_page(
        &self,
        after: Option<&DownloadsPageCursor>,
        limit: usize,
    ) -> Result<DownloadsPage, RepositoryError>;
    fn list_downloads_by_status(
        &self,
        status: DownloadStatus,
    ) -> Result<Vec<Download>, RepositoryError>;
    fn update_download(&self, download: &Download) -> Result<(), RepositoryError>;
    fn delete_download(&self, info_hash: &str) -> Result<(), RepositoryError>;
}

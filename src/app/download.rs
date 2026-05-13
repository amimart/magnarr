use thiserror::Error;

use crate::types::{Download, DownloadStatus};

pub const DEFAULT_DOWNLOADS_PAGE_SIZE: usize = 50;
pub const MAX_DOWNLOADS_PAGE_SIZE: usize = 100;

pub type DownloadIter = Box<dyn Iterator<Item = Download> + Send>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DownloadListOrder {
    CreatedAtAsc,
    #[default]
    CreatedAtDesc,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DownloadListQuery {
    pub status: Option<DownloadStatus>,
    pub order: Option<DownloadListOrder>,
    pub from_created_at: Option<chrono::DateTime<chrono::Utc>>,
    pub after_info_hash: Option<String>,
}

impl DownloadListQuery {
    pub fn order(&self) -> DownloadListOrder {
        self.order.unwrap_or_default()
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
    fn list_downloads(&self, query: &DownloadListQuery) -> Result<DownloadIter, RepositoryError>;
    fn update_download(&self, download: &Download) -> Result<(), RepositoryError>;
    fn delete_download(&self, info_hash: &str) -> Result<(), RepositoryError>;
}

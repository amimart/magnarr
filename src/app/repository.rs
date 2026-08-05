use thiserror::Error;

use crate::types::{Download, DownloadStatus};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortOrder {
    Asc,
    #[default]
    Desc,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadCursor(Vec<u8>);

impl DownloadCursor {
    pub fn new(raw: Vec<u8>) -> Self {
        DownloadCursor(raw)
    }
}

impl AsRef<[u8]> for DownloadCursor {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadEntry {
    pub download: Download,
    pub cursor: DownloadCursor,
}

#[derive(Debug, Error)]
pub enum RepositoryError {
    #[error("not found")]
    NotFound,
    #[error("already exists")]
    AlreadyExists,
    #[error("invalid cursor")]
    InvalidCursor,
    #[error("backend error: {0}")]
    Storage(String),
    #[error("serialization error: {0}")]
    Serde(String),
}

pub trait DownloadRepository: Send + Sync {
    type Iter<'a>: Iterator<Item = Result<DownloadEntry, RepositoryError>> + 'a
    where
        Self: 'a;

    fn insert(&self, download: &Download) -> Result<(), RepositoryError>;
    fn get(&self, info_hash: &str) -> Result<Download, RepositoryError>;
    fn scan_all(
        &self,
        after: Option<DownloadCursor>,
        order: SortOrder,
    ) -> Result<Self::Iter<'_>, RepositoryError>;
    fn scan_by_status(
        &self,
        status: DownloadStatus,
        after: Option<DownloadCursor>,
        order: SortOrder,
    ) -> Result<Self::Iter<'_>, RepositoryError>;
    fn scan_since(
        &self,
        since: chrono::DateTime<chrono::Utc>,
        after: Option<DownloadCursor>,
        order: SortOrder,
    ) -> Result<Self::Iter<'_>, RepositoryError>;
    fn scan_by_status_since(
        &self,
        status: DownloadStatus,
        since: chrono::DateTime<chrono::Utc>,
        after: Option<DownloadCursor>,
        order: SortOrder,
    ) -> Result<Self::Iter<'_>, RepositoryError>;
    fn update(&self, download: &Download) -> Result<(), RepositoryError>;
    fn remove(&self, info_hash: &str) -> Result<(), RepositoryError>;
}

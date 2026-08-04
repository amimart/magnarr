use collette::iter::Entry;
use collette::{Cursor, Direction};
use thiserror::Error;

use crate::types::{Download, DownloadStatus};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortOrder {
    Asc,
    #[default]
    Desc,
}

impl From<SortOrder> for Direction {
    fn from(val: SortOrder) -> Self {
        match val {
            SortOrder::Asc => Direction::LeftToRight,
            SortOrder::Desc => Direction::RightToLeft,
        }
    }
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

impl From<Cursor> for DownloadCursor {
    fn from(cursor: Cursor) -> Self {
        match cursor {
            Cursor::None => DownloadCursor(Vec::new()),
            Cursor::Key(k) => DownloadCursor(k.to_vec()),
        }
    }
}

impl From<DownloadCursor> for Cursor {
    fn from(value: DownloadCursor) -> Self {
        Cursor::Key(value.0.into())
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
    type Iter<'a>: Iterator<Item = Result<Entry<Download>, RepositoryError>> + 'a
    where
        Self: 'a;

    fn insert(&self, download: &Download) -> Result<(), RepositoryError>;
    fn get(&self, info_hash: &str) -> Result<Download, RepositoryError>;
    fn list(
        &self,
        status: Option<DownloadStatus>,
        from: Option<chrono::DateTime<chrono::Utc>>,
        after: Cursor,
        order: SortOrder,
    ) -> Result<Self::Iter<'_>, RepositoryError>;
    fn update(&self, download: &Download) -> Result<(), RepositoryError>;
    fn remove(&self, info_hash: &str) -> Result<(), RepositoryError>;
}

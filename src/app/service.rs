use crate::app::error::AppError;
use crate::app::repository::{DownloadCursor, DownloadEntry, RepositoryError, SortOrder};
use crate::types::{Download, DownloadStatus, Magnet};
use async_trait::async_trait;

#[async_trait]
pub trait DownloadService: Send + Sync {
    type Iter<'a>: Iterator<Item = Result<DownloadEntry, AppError>> + 'a
    where
        Self: 'a;

    /// Submits a new download: persists it as `Queued`, sends the magnet to the
    /// torrent client, then transitions to `Submitted`. If the client rejects the
    /// magnet the record is deleted (rollback) and an error is returned.
    async fn download(&self, magnet: Magnet, target_dir: String) -> Result<Download, AppError>;

    fn downloads(
        &self,
        status: Option<DownloadStatus>,
        from: Option<chrono::DateTime<chrono::Utc>>,
        after: Option<DownloadCursor>,
        order: SortOrder,
    ) -> Result<Self::Iter<'_>, AppError>;
}

pub struct DownloadIter<I> {
    inner: I,
}

impl<I> DownloadIter<I> {
    pub fn new(inner: I) -> Self {
        Self { inner }
    }
}

impl<I> Iterator for DownloadIter<I>
where
    I: Iterator<Item = Result<DownloadEntry, RepositoryError>>,
{
    type Item = Result<DownloadEntry, AppError>;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner
            .next()
            .map(|result| result.map_err(AppError::from))
    }
}

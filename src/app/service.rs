use crate::app::download::{DownloadCursor, SortOrder};
use crate::app::error::AppError;
use crate::types::{Download, DownloadStatus, Magnet};
use async_trait::async_trait;

#[async_trait]
pub trait DownloadService: Send + Sync {
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
    ) -> Result<Box<dyn Iterator<Item = Result<Download, AppError>> + '_>, AppError>;
}

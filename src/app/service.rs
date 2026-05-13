use async_trait::async_trait;
use crate::app::download::{DownloadCursor, DownloadListOrder};
use crate::app::error::AppError;
use crate::types::{Download, DownloadStatus, Magnet};

#[async_trait]
pub trait DownloadService: Send + Sync {
    async fn download(&self, magnet: Magnet, target_dir: String) -> Result<Download, AppError>;

    fn downloads(
        &self,
        status: Option<DownloadStatus>,
        from: Option<chrono::DateTime<chrono::Utc>>,
        after: Option<DownloadCursor>,
        order: DownloadListOrder,
    ) -> Result<Box<dyn Iterator<Item=Result<Download, AppError>> + '_>, AppError>;
}
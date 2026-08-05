use async_graphql::{Enum, Object};
use chrono::{DateTime, Utc};

use crate::app::repository::SortOrder as AppSortOrder;
use crate::graphql::scalars::MagnetUri;
use crate::types::{Download as DomainDownload, DownloadStatus as DomainDownloadStatus};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Enum)]
pub enum DownloadStatus {
    Queued,
    Submitted,
    Downloading,
    Importing,
    Imported,
    Failed,
}

impl From<DomainDownloadStatus> for DownloadStatus {
    fn from(s: DomainDownloadStatus) -> Self {
        match s {
            DomainDownloadStatus::Queued => Self::Queued,
            DomainDownloadStatus::Submitted => Self::Submitted,
            DomainDownloadStatus::Downloading => Self::Downloading,
            DomainDownloadStatus::Importing => Self::Importing,
            DomainDownloadStatus::Imported => Self::Imported,
            DomainDownloadStatus::Failed => Self::Failed,
        }
    }
}

impl From<DownloadStatus> for DomainDownloadStatus {
    fn from(s: DownloadStatus) -> Self {
        match s {
            DownloadStatus::Queued => Self::Queued,
            DownloadStatus::Submitted => Self::Submitted,
            DownloadStatus::Downloading => Self::Downloading,
            DownloadStatus::Importing => Self::Importing,
            DownloadStatus::Imported => Self::Imported,
            DownloadStatus::Failed => Self::Failed,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Enum)]
pub enum SortOrder {
    Asc,
    Desc,
}

impl From<SortOrder> for AppSortOrder {
    fn from(s: SortOrder) -> Self {
        match s {
            SortOrder::Asc => Self::Asc,
            SortOrder::Desc => Self::Desc,
        }
    }
}

pub struct Download(DomainDownload);

impl From<DomainDownload> for Download {
    fn from(d: DomainDownload) -> Self {
        Self(d)
    }
}

#[Object]
impl Download {
    async fn info_hash(&self) -> &str {
        &self.0.info_hash
    }

    async fn name(&self) -> &str {
        &self.0.name
    }

    async fn content_name(&self) -> &str {
        &self.0.content_name
    }

    async fn magnet(&self) -> MagnetUri {
        MagnetUri(self.0.magnet.clone())
    }

    async fn status(&self) -> DownloadStatus {
        self.0.status.into()
    }

    async fn target_dir(&self) -> &str {
        &self.0.target_dir
    }

    async fn created_at(&self) -> DateTime<Utc> {
        self.0.created_at
    }

    async fn updated_at(&self) -> DateTime<Utc> {
        self.0.updated_at
    }

    async fn imported_path(&self) -> Option<&str> {
        self.0.imported_path.as_deref()
    }

    async fn error(&self) -> Option<&str> {
        self.0.error.as_deref()
    }
}

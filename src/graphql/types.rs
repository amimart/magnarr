use async_graphql::{Enum, Object};
use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::types::{Download as DomainDownload, DownloadStatus as DomainDownloadStatus};
use crate::graphql::scalars::MagnetUri;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Enum)]
pub enum DownloadStatus {
    Queued,
    Submitted,
    Downloading,
    Completed,
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
            DomainDownloadStatus::Completed => Self::Completed,
            DomainDownloadStatus::Importing => Self::Importing,
            DomainDownloadStatus::Imported => Self::Imported,
            DomainDownloadStatus::Failed => Self::Failed,
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
    async fn id(&self) -> Uuid {
        self.0.id
    }

    async fn magnet_uri(&self) -> MagnetUri {
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

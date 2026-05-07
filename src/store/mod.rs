pub mod redb;

use thiserror::Error;

use crate::model::Download;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("not found")]
    NotFound,
    #[error("already exists")]
    AlreadyExists,
    #[error("backend error: {0}")]
    Backend(String),
    #[error("serialization error: {0}")]
    Serialization(String),
}

pub trait Store: Send + Sync {
    fn create_download(&self, download: &Download) -> Result<(), StoreError>;
    fn get_download(&self, id: uuid::Uuid) -> Result<Download, StoreError>;
    fn list_downloads(&self) -> Result<Vec<Download>, StoreError>;
    fn update_download(&self, download: &Download) -> Result<(), StoreError>;
    fn delete_download(&self, id: uuid::Uuid) -> Result<(), StoreError>;
}

use redb::{Database, ReadableTable, TableDefinition};

use crate::app::download::{DownloadRepository, RepositoryError};
use crate::app::model::{Download, DownloadStatus};

const DOWNLOADS: TableDefinition<&str, &str> = TableDefinition::new("downloads");
const INDEXES: TableDefinition<&str, &str> = TableDefinition::new("indexes");
/// Key: `{status}:{uuid}`, value: uuid. Enables O(log n) lookup by status.
const STATUS_INDEX: TableDefinition<&str, &str> = TableDefinition::new("status_index");

fn status_prefix(status: DownloadStatus) -> &'static str {
    match status {
        DownloadStatus::Queued => "queued",
        DownloadStatus::Submitted => "submitted",
        DownloadStatus::Downloading => "downloading",
        DownloadStatus::Completed => "completed",
        DownloadStatus::Importing => "importing",
        DownloadStatus::Imported => "imported",
        DownloadStatus::Failed => "failed",
    }
}

pub struct RedbStore {
    db: Database,
}

impl RedbStore {
    pub fn new(path: &str) -> Result<Self, RepositoryError> {
        if let Some(parent) = std::path::Path::new(path).parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| RepositoryError::Backend(e.to_string()))?;
            }
        }

        let db = Database::create(path).map_err(|e| RepositoryError::Backend(e.to_string()))?;

        // Ensure tables exist.
        let tx = db
            .begin_write()
            .map_err(|e| RepositoryError::Backend(e.to_string()))?;
        {
            tx.open_table(DOWNLOADS)
                .map_err(|e| RepositoryError::Backend(e.to_string()))?;
            tx.open_table(INDEXES)
                .map_err(|e| RepositoryError::Backend(e.to_string()))?;
            tx.open_table(STATUS_INDEX)
                .map_err(|e| RepositoryError::Backend(e.to_string()))?;
        }
        tx.commit()
            .map_err(|e| RepositoryError::Backend(e.to_string()))?;

        Ok(Self { db })
    }
}

impl DownloadRepository for RedbStore {
    fn create_download(&self, download: &Download) -> Result<(), RepositoryError> {
        let id_str = download.id.to_string();
        let json = serde_json::to_string(download)
            .map_err(|e| RepositoryError::Serialization(e.to_string()))?;

        let tx = self
            .db
            .begin_write()
            .map_err(|e| RepositoryError::Backend(e.to_string()))?;
        {
            let mut table = tx
                .open_table(DOWNLOADS)
                .map_err(|e| RepositoryError::Backend(e.to_string()))?;

            if table
                .get(id_str.as_str())
                .map_err(|e| RepositoryError::Backend(e.to_string()))?
                .is_some()
            {
                return Err(RepositoryError::AlreadyExists);
            }

            table
                .insert(id_str.as_str(), json.as_str())
                .map_err(|e| RepositoryError::Backend(e.to_string()))?;

            if let Some(ref hash) = download.info_hash {
                let index_key = format!("infohash:{hash}");
                let mut indexes = tx
                    .open_table(INDEXES)
                    .map_err(|e| RepositoryError::Backend(e.to_string()))?;
                indexes
                    .insert(index_key.as_str(), id_str.as_str())
                    .map_err(|e| RepositoryError::Backend(e.to_string()))?;
            }

            let status_key = format!("{}:{id_str}", status_prefix(download.status));
            let mut status_idx = tx
                .open_table(STATUS_INDEX)
                .map_err(|e| RepositoryError::Backend(e.to_string()))?;
            status_idx
                .insert(status_key.as_str(), id_str.as_str())
                .map_err(|e| RepositoryError::Backend(e.to_string()))?;
        }
        tx.commit()
            .map_err(|e| RepositoryError::Backend(e.to_string()))?;
        Ok(())
    }

    fn find_by_info_hash(&self, info_hash: &str) -> Result<Option<Download>, RepositoryError> {
        let index_key = format!("infohash:{info_hash}");
        let tx = self
            .db
            .begin_read()
            .map_err(|e| RepositoryError::Backend(e.to_string()))?;

        let id_str = {
            let indexes = tx
                .open_table(INDEXES)
                .map_err(|e| RepositoryError::Backend(e.to_string()))?;
            indexes
                .get(index_key.as_str())
                .map_err(|e| RepositoryError::Backend(e.to_string()))?
                .map(|v| v.value().to_owned())
        };

        let Some(id_str) = id_str else {
            return Ok(None);
        };

        let table = tx
            .open_table(DOWNLOADS)
            .map_err(|e| RepositoryError::Backend(e.to_string()))?;
        let Some(entry) = table
            .get(id_str.as_str())
            .map_err(|e| RepositoryError::Backend(e.to_string()))?
        else {
            return Ok(None);
        };
        serde_json::from_str(entry.value())
            .map_err(|e| RepositoryError::Serialization(e.to_string()))
            .map(Some)
    }

    fn get_download(&self, id: uuid::Uuid) -> Result<Download, RepositoryError> {
        let id_str = id.to_string();
        let tx = self
            .db
            .begin_read()
            .map_err(|e| RepositoryError::Backend(e.to_string()))?;
        let table = tx
            .open_table(DOWNLOADS)
            .map_err(|e| RepositoryError::Backend(e.to_string()))?;
        let entry = table
            .get(id_str.as_str())
            .map_err(|e| RepositoryError::Backend(e.to_string()))?
            .ok_or(RepositoryError::NotFound)?;
        serde_json::from_str(entry.value())
            .map_err(|e| RepositoryError::Serialization(e.to_string()))
    }

    fn list_downloads(&self) -> Result<Vec<Download>, RepositoryError> {
        let tx = self
            .db
            .begin_read()
            .map_err(|e| RepositoryError::Backend(e.to_string()))?;
        let table = tx
            .open_table(DOWNLOADS)
            .map_err(|e| RepositoryError::Backend(e.to_string()))?;
        let mut downloads = Vec::new();
        for entry in table
            .iter()
            .map_err(|e| RepositoryError::Backend(e.to_string()))?
        {
            let (_, value) =
                entry.map_err(|e: redb::StorageError| RepositoryError::Backend(e.to_string()))?;
            let dl: Download = serde_json::from_str(value.value())
                .map_err(|e| RepositoryError::Serialization(e.to_string()))?;
            downloads.push(dl);
        }
        Ok(downloads)
    }

    fn update_download(&self, download: &Download) -> Result<(), RepositoryError> {
        let id_str = download.id.to_string();
        let json = serde_json::to_string(download)
            .map_err(|e| RepositoryError::Serialization(e.to_string()))?;

        let tx = self
            .db
            .begin_write()
            .map_err(|e| RepositoryError::Backend(e.to_string()))?;
        {
            let mut table = tx
                .open_table(DOWNLOADS)
                .map_err(|e| RepositoryError::Backend(e.to_string()))?;

            // Read old status before overwriting so we can update the index.
            let old_status = {
                let entry = table
                    .get(id_str.as_str())
                    .map_err(|e| RepositoryError::Backend(e.to_string()))?;
                entry
                    .map(|v| {
                        serde_json::from_str::<Download>(v.value())
                            .map(|d| d.status)
                            .map_err(|e| RepositoryError::Serialization(e.to_string()))
                    })
                    .transpose()?
            };

            table
                .insert(id_str.as_str(), json.as_str())
                .map_err(|e| RepositoryError::Backend(e.to_string()))?;

            // Update the status index only when the status actually changed.
            if old_status.as_ref() != Some(&download.status) {
                let mut status_idx = tx
                    .open_table(STATUS_INDEX)
                    .map_err(|e| RepositoryError::Backend(e.to_string()))?;
                if let Some(old) = old_status {
                    let old_key = format!("{}:{id_str}", status_prefix(old));
                    status_idx
                        .remove(old_key.as_str())
                        .map_err(|e| RepositoryError::Backend(e.to_string()))?;
                }
                let new_key = format!("{}:{id_str}", status_prefix(download.status));
                status_idx
                    .insert(new_key.as_str(), id_str.as_str())
                    .map_err(|e| RepositoryError::Backend(e.to_string()))?;
            }
        }
        tx.commit()
            .map_err(|e| RepositoryError::Backend(e.to_string()))?;
        Ok(())
    }

    fn delete_download(&self, id: uuid::Uuid) -> Result<(), RepositoryError> {
        let id_str = id.to_string();

        let tx = self
            .db
            .begin_write()
            .map_err(|e| RepositoryError::Backend(e.to_string()))?;
        {
            let mut table = tx
                .open_table(DOWNLOADS)
                .map_err(|e| RepositoryError::Backend(e.to_string()))?;

            let entry = table
                .get(id_str.as_str())
                .map_err(|e| RepositoryError::Backend(e.to_string()))?
                .ok_or(RepositoryError::NotFound)?;

            let download: Download = serde_json::from_str(entry.value())
                .map_err(|e| RepositoryError::Serialization(e.to_string()))?;

            // Drop entry guard before mutating the table.
            drop(entry);

            table
                .remove(id_str.as_str())
                .map_err(|e| RepositoryError::Backend(e.to_string()))?;

            if let Some(ref hash) = download.info_hash {
                let index_key = format!("infohash:{hash}");
                let mut indexes = tx
                    .open_table(INDEXES)
                    .map_err(|e| RepositoryError::Backend(e.to_string()))?;
                indexes
                    .remove(index_key.as_str())
                    .map_err(|e| RepositoryError::Backend(e.to_string()))?;
            }

            let status_key = format!("{}:{id_str}", status_prefix(download.status));
            let mut status_idx = tx
                .open_table(STATUS_INDEX)
                .map_err(|e| RepositoryError::Backend(e.to_string()))?;
            status_idx
                .remove(status_key.as_str())
                .map_err(|e| RepositoryError::Backend(e.to_string()))?;
        }
        tx.commit()
            .map_err(|e| RepositoryError::Backend(e.to_string()))?;
        Ok(())
    }

    fn list_downloads_by_status(
        &self,
        status: crate::app::model::DownloadStatus,
    ) -> Result<Vec<Download>, RepositoryError> {
        let prefix = status_prefix(status);
        // `;` (0x3B) is the next ASCII char after `:` (0x3A); UUID chars are all below it.
        let start = format!("{prefix}:");
        let end = format!("{prefix};");

        let tx = self
            .db
            .begin_read()
            .map_err(|e| RepositoryError::Backend(e.to_string()))?;

        let ids: Vec<String> = {
            let idx = tx
                .open_table(STATUS_INDEX)
                .map_err(|e| RepositoryError::Backend(e.to_string()))?;
            idx.range(start.as_str()..end.as_str())
                .map_err(|e| RepositoryError::Backend(e.to_string()))?
                .map(|e| {
                    e.map(|(_, v)| v.value().to_owned())
                        .map_err(|err: redb::StorageError| {
                            RepositoryError::Backend(err.to_string())
                        })
                })
                .collect::<Result<Vec<_>, _>>()?
        };

        let table = tx
            .open_table(DOWNLOADS)
            .map_err(|e| RepositoryError::Backend(e.to_string()))?;
        let mut results = Vec::with_capacity(ids.len());
        for id in &ids {
            if let Some(entry) = table
                .get(id.as_str())
                .map_err(|e| RepositoryError::Backend(e.to_string()))?
            {
                let dl: Download = serde_json::from_str(entry.value())
                    .map_err(|e| RepositoryError::Serialization(e.to_string()))?;
                results.push(dl);
            }
        }
        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::model::{Download, DownloadStatus};

    const MAGNET: &str = "magnet:?xt=urn:btih:ABCDEF1234567890ABCDEF1234567890ABCDEF12&dn=test";

    fn new_store() -> (RedbStore, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.redb");
        let store = RedbStore::new(path.to_str().unwrap()).unwrap();
        (store, dir)
    }

    fn test_download() -> Download {
        let uri = MAGNET.parse().unwrap();
        Download::new(uri, "/downloads".to_owned())
    }

    #[test]
    fn create_then_get_returns_same_download() {
        let (store, _dir) = new_store();
        let dl = test_download();
        store.create_download(&dl).unwrap();
        let fetched = store.get_download(dl.id).unwrap();
        assert_eq!(fetched.id, dl.id);
        assert_eq!(fetched.magnet_uri, dl.magnet_uri);
        assert_eq!(fetched.status, dl.status);
    }

    #[test]
    fn create_twice_same_id_returns_already_exists() {
        let (store, _dir) = new_store();
        let dl = test_download();
        store.create_download(&dl).unwrap();
        let result = store.create_download(&dl);
        assert!(matches!(result, Err(RepositoryError::AlreadyExists)));
    }

    #[test]
    fn get_unknown_uuid_returns_not_found() {
        let (store, _dir) = new_store();
        let result = store.get_download(uuid::Uuid::new_v4());
        assert!(matches!(result, Err(RepositoryError::NotFound)));
    }

    #[test]
    fn list_downloads_returns_all_created() {
        let (store, _dir) = new_store();
        let dl1 = test_download();
        let dl2 = test_download();
        store.create_download(&dl1).unwrap();
        store.create_download(&dl2).unwrap();

        let list = store.list_downloads().unwrap();
        assert_eq!(list.len(), 2);
        assert!(list.iter().any(|d| d.id == dl1.id));
        assert!(list.iter().any(|d| d.id == dl2.id));
    }

    #[test]
    fn update_download_persists_changes() {
        let (store, _dir) = new_store();
        let dl = test_download();
        store.create_download(&dl).unwrap();

        let mut updated = dl.clone();
        updated.status = DownloadStatus::Downloading;
        updated.touch();
        store.update_download(&updated).unwrap();

        let fetched = store.get_download(dl.id).unwrap();
        assert_eq!(fetched.status, DownloadStatus::Downloading);
        assert!(fetched.updated_at >= dl.updated_at);
    }

    #[test]
    fn find_by_info_hash_returns_download() {
        let (store, _dir) = new_store();
        let dl = test_download();
        let hash = dl.info_hash.as_deref().unwrap().to_owned();
        store.create_download(&dl).unwrap();

        let found = store.find_by_info_hash(&hash).unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().id, dl.id);
    }

    #[test]
    fn find_by_info_hash_returns_none_for_unknown_hash() {
        let (store, _dir) = new_store();
        let result = store.find_by_info_hash("0000000000000000000000000000000000000000");
        assert!(matches!(result, Ok(None)));
    }

    #[test]
    fn delete_download_removes_it() {
        let (store, _dir) = new_store();
        let dl = test_download();
        store.create_download(&dl).unwrap();
        store.delete_download(dl.id).unwrap();

        let result = store.get_download(dl.id);
        assert!(matches!(result, Err(RepositoryError::NotFound)));
    }

    #[test]
    fn list_downloads_by_status_returns_only_matching() {
        let (store, _dir) = new_store();

        let dl1 = test_download(); // Queued
        let mut dl2 = test_download();
        dl2.status = DownloadStatus::Submitted;
        let mut dl3 = test_download();
        dl3.status = DownloadStatus::Downloading;

        store.create_download(&dl1).unwrap();
        store.create_download(&dl2).unwrap();
        store.create_download(&dl3).unwrap();

        let queued = store.list_downloads_by_status(DownloadStatus::Queued).unwrap();
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0].id, dl1.id);

        let submitted = store.list_downloads_by_status(DownloadStatus::Submitted).unwrap();
        assert_eq!(submitted.len(), 1);
        assert_eq!(submitted[0].id, dl2.id);
    }

    #[test]
    fn list_downloads_by_status_reflects_updates() {
        let (store, _dir) = new_store();
        let dl = test_download(); // Queued
        store.create_download(&dl).unwrap();

        let mut updated = dl.clone();
        updated.status = DownloadStatus::Submitted;
        updated.touch();
        store.update_download(&updated).unwrap();

        let queued = store.list_downloads_by_status(DownloadStatus::Queued).unwrap();
        assert!(queued.is_empty(), "should no longer appear as Queued");

        let submitted = store.list_downloads_by_status(DownloadStatus::Submitted).unwrap();
        assert_eq!(submitted.len(), 1);
        assert_eq!(submitted[0].id, dl.id);
    }

    #[test]
    fn delete_download_removes_status_index_entry() {
        let (store, _dir) = new_store();
        let dl = test_download();
        store.create_download(&dl).unwrap();
        store.delete_download(dl.id).unwrap();

        let results = store.list_downloads_by_status(DownloadStatus::Queued).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn delete_download_removes_infohash_index() {
        let (store, _dir) = new_store();
        let dl = test_download();
        assert!(dl.info_hash.is_some(), "test download must have info_hash");
        store.create_download(&dl).unwrap();
        store.delete_download(dl.id).unwrap();

        let tx = store.db.begin_read().unwrap();
        let indexes = tx.open_table(INDEXES).unwrap();
        let hash = dl.info_hash.as_deref().unwrap();
        let key = format!("infohash:{hash}");
        assert!(indexes.get(key.as_str()).unwrap().is_none());
    }
}

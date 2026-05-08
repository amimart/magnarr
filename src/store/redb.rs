use redb::{Database, ReadableTable, TableDefinition};

use crate::app::download::{DownloadRepository, RepositoryError};
use crate::app::model::Download;

const DOWNLOADS: TableDefinition<&str, &str> = TableDefinition::new("downloads");
const INDEXES: TableDefinition<&str, &str> = TableDefinition::new("indexes");

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
        }
        tx.commit()
            .map_err(|e| RepositoryError::Backend(e.to_string()))?;
        Ok(())
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
        let mut updated = download.clone();
        updated.updated_at = chrono::Utc::now();

        let json = serde_json::to_string(&updated)
            .map_err(|e| RepositoryError::Serialization(e.to_string()))?;

        let tx = self
            .db
            .begin_write()
            .map_err(|e| RepositoryError::Backend(e.to_string()))?;
        {
            let mut table = tx
                .open_table(DOWNLOADS)
                .map_err(|e| RepositoryError::Backend(e.to_string()))?;
            table
                .insert(id_str.as_str(), json.as_str())
                .map_err(|e| RepositoryError::Backend(e.to_string()))?;
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
        }
        tx.commit()
            .map_err(|e| RepositoryError::Backend(e.to_string()))?;
        Ok(())
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
    fn update_download_changes_updated_at_and_persists() {
        let (store, _dir) = new_store();
        let dl = test_download();
        store.create_download(&dl).unwrap();

        let mut updated = dl.clone();
        updated.status = DownloadStatus::Downloading;
        store.update_download(&updated).unwrap();

        let fetched = store.get_download(dl.id).unwrap();
        assert_eq!(fetched.status, DownloadStatus::Downloading);
        assert!(fetched.updated_at >= dl.updated_at);
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

use redb::{Database, ReadableTable, TableDefinition};

use crate::app::download::{DownloadRepository, RepositoryError};
use crate::types::{Download, DownloadStatus};

const DOWNLOADS: TableDefinition<&str, &str> = TableDefinition::new("downloads");
/// Key: `{status}:{info_hash}`, value: info_hash. Enables O(log n) lookup by status.
/// `;` (0x3B) is the next ASCII char after `:` (0x3A); info_hash hex chars are all below it.
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

        let tx = db
            .begin_write()
            .map_err(|e| RepositoryError::Backend(e.to_string()))?;
        {
            tx.open_table(DOWNLOADS)
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
                .get(download.info_hash.as_str())
                .map_err(|e| RepositoryError::Backend(e.to_string()))?
                .is_some()
            {
                return Err(RepositoryError::AlreadyExists);
            }

            table
                .insert(download.info_hash.as_str(), json.as_str())
                .map_err(|e| RepositoryError::Backend(e.to_string()))?;

            let status_key = format!("{}:{}", status_prefix(download.status), download.info_hash);
            let mut status_idx = tx
                .open_table(STATUS_INDEX)
                .map_err(|e| RepositoryError::Backend(e.to_string()))?;
            status_idx
                .insert(status_key.as_str(), download.info_hash.as_str())
                .map_err(|e| RepositoryError::Backend(e.to_string()))?;
        }
        tx.commit()
            .map_err(|e| RepositoryError::Backend(e.to_string()))?;
        Ok(())
    }

    fn find_by_info_hash(&self, info_hash: &str) -> Result<Download, RepositoryError> {
        let tx = self
            .db
            .begin_read()
            .map_err(|e| RepositoryError::Backend(e.to_string()))?;
        let table = tx
            .open_table(DOWNLOADS)
            .map_err(|e| RepositoryError::Backend(e.to_string()))?;
        let entry = table
            .get(info_hash)
            .map_err(|e| RepositoryError::Backend(e.to_string()))?
            .ok_or(RepositoryError::NotFound)?;
        serde_json::from_str(entry.value())
            .map_err(|e| RepositoryError::Serialization(e.to_string()))
    }

    fn list_downloads_by_status(
        &self,
        status: DownloadStatus,
    ) -> Result<Vec<Download>, RepositoryError> {
        let prefix = status_prefix(status);
        let start = format!("{prefix}:");
        let end = format!("{prefix};");

        let tx = self
            .db
            .begin_read()
            .map_err(|e| RepositoryError::Backend(e.to_string()))?;

        let hashes: Vec<String> = {
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
        let mut results = Vec::with_capacity(hashes.len());
        for hash in &hashes {
            if let Some(entry) = table
                .get(hash.as_str())
                .map_err(|e| RepositoryError::Backend(e.to_string()))?
            {
                let dl: Download = serde_json::from_str(entry.value())
                    .map_err(|e| RepositoryError::Serialization(e.to_string()))?;
                results.push(dl);
            }
        }
        Ok(results)
    }

    fn update_download(&self, download: &Download) -> Result<(), RepositoryError> {
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
                    .get(download.info_hash.as_str())
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
                .insert(download.info_hash.as_str(), json.as_str())
                .map_err(|e| RepositoryError::Backend(e.to_string()))?;

            // Update the status index only when the status actually changed.
            if old_status.as_ref() != Some(&download.status) {
                let mut status_idx = tx
                    .open_table(STATUS_INDEX)
                    .map_err(|e| RepositoryError::Backend(e.to_string()))?;
                if let Some(old) = old_status {
                    let old_key = format!("{}:{}", status_prefix(old), download.info_hash);
                    status_idx
                        .remove(old_key.as_str())
                        .map_err(|e| RepositoryError::Backend(e.to_string()))?;
                }
                let new_key = format!("{}:{}", status_prefix(download.status), download.info_hash);
                status_idx
                    .insert(new_key.as_str(), download.info_hash.as_str())
                    .map_err(|e| RepositoryError::Backend(e.to_string()))?;
            }
        }
        tx.commit()
            .map_err(|e| RepositoryError::Backend(e.to_string()))?;
        Ok(())
    }

    fn delete_download(&self, info_hash: &str) -> Result<(), RepositoryError> {
        let tx = self
            .db
            .begin_write()
            .map_err(|e| RepositoryError::Backend(e.to_string()))?;
        {
            let mut table = tx
                .open_table(DOWNLOADS)
                .map_err(|e| RepositoryError::Backend(e.to_string()))?;

            let entry = table
                .get(info_hash)
                .map_err(|e| RepositoryError::Backend(e.to_string()))?
                .ok_or(RepositoryError::NotFound)?;

            let download: Download = serde_json::from_str(entry.value())
                .map_err(|e| RepositoryError::Serialization(e.to_string()))?;

            // Drop entry guard before mutating the table.
            drop(entry);

            table
                .remove(info_hash)
                .map_err(|e| RepositoryError::Backend(e.to_string()))?;

            let status_key = format!("{}:{info_hash}", status_prefix(download.status));
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
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAGNET1: &str = "magnet:?xt=urn:btih:ABCDEF1234567890ABCDEF1234567890ABCDEF12&dn=test1";
    const MAGNET2: &str = "magnet:?xt=urn:btih:FEDCBA0987654321FEDCBA0987654321FEDCBA09&dn=test2";
    const MAGNET3: &str = "magnet:?xt=urn:btih:1111111111111111111111111111111111111111&dn=test3";

    fn new_store() -> (RedbStore, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.redb");
        let store = RedbStore::new(path.to_str().unwrap()).unwrap();
        (store, dir)
    }

    fn make_download(magnet_str: &str) -> Download {
        let magnet = magnet_str.parse().unwrap();
        Download::new(magnet, "/downloads".to_owned())
    }

    #[test]
    fn create_then_find_returns_same_download() {
        let (store, _dir) = new_store();
        let dl = make_download(MAGNET1);
        store.create_download(&dl).unwrap();

        let fetched = store.find_by_info_hash(&dl.info_hash).unwrap();
        assert_eq!(fetched.info_hash, dl.info_hash);
        assert_eq!(fetched.magnet, dl.magnet);
        assert_eq!(fetched.status, dl.status);
    }

    #[test]
    fn create_twice_same_info_hash_returns_already_exists() {
        let (store, _dir) = new_store();
        let dl = make_download(MAGNET1);
        store.create_download(&dl).unwrap();
        let result = store.create_download(&dl);
        assert!(matches!(result, Err(RepositoryError::AlreadyExists)));
    }

    #[test]
    fn find_unknown_info_hash_returns_not_found() {
        let (store, _dir) = new_store();
        let result = store.find_by_info_hash("0000000000000000000000000000000000000000");
        assert!(matches!(result, Err(RepositoryError::NotFound)));
    }

    #[test]
    fn list_downloads_by_status_returns_all_matching() {
        let (store, _dir) = new_store();
        let dl1 = make_download(MAGNET1);
        let dl2 = make_download(MAGNET2);
        store.create_download(&dl1).unwrap();
        store.create_download(&dl2).unwrap();

        let queued = store
            .list_downloads_by_status(DownloadStatus::Queued)
            .unwrap();
        assert_eq!(queued.len(), 2);
        assert!(queued.iter().any(|d| d.info_hash == dl1.info_hash));
        assert!(queued.iter().any(|d| d.info_hash == dl2.info_hash));
    }

    #[test]
    fn update_download_persists_changes() {
        let (store, _dir) = new_store();
        let dl = make_download(MAGNET1);
        store.create_download(&dl).unwrap();

        let mut updated = dl.clone();
        updated.status = DownloadStatus::Downloading;
        updated.touch();
        store.update_download(&updated).unwrap();

        let fetched = store.find_by_info_hash(&dl.info_hash).unwrap();
        assert_eq!(fetched.status, DownloadStatus::Downloading);
        assert!(fetched.updated_at >= dl.updated_at);
    }

    #[test]
    fn delete_download_removes_it() {
        let (store, _dir) = new_store();
        let dl = make_download(MAGNET1);
        store.create_download(&dl).unwrap();
        store.delete_download(&dl.info_hash).unwrap();

        let result = store.find_by_info_hash(&dl.info_hash);
        assert!(matches!(result, Err(RepositoryError::NotFound)));
    }

    #[test]
    fn list_downloads_by_status_returns_only_matching() {
        let (store, _dir) = new_store();

        let dl1 = make_download(MAGNET1); // Queued
        let mut dl2 = make_download(MAGNET2);
        dl2.status = DownloadStatus::Submitted;
        let mut dl3 = make_download(MAGNET3);
        dl3.status = DownloadStatus::Downloading;

        store.create_download(&dl1).unwrap();
        store.create_download(&dl2).unwrap();
        store.create_download(&dl3).unwrap();

        let queued = store
            .list_downloads_by_status(DownloadStatus::Queued)
            .unwrap();
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0].info_hash, dl1.info_hash);

        let submitted = store
            .list_downloads_by_status(DownloadStatus::Submitted)
            .unwrap();
        assert_eq!(submitted.len(), 1);
        assert_eq!(submitted[0].info_hash, dl2.info_hash);
    }

    #[test]
    fn list_downloads_by_status_reflects_updates() {
        let (store, _dir) = new_store();
        let dl = make_download(MAGNET1);
        store.create_download(&dl).unwrap();

        let mut updated = dl.clone();
        updated.status = DownloadStatus::Submitted;
        updated.touch();
        store.update_download(&updated).unwrap();

        let queued = store
            .list_downloads_by_status(DownloadStatus::Queued)
            .unwrap();
        assert!(queued.is_empty(), "should no longer appear as Queued");

        let submitted = store
            .list_downloads_by_status(DownloadStatus::Submitted)
            .unwrap();
        assert_eq!(submitted.len(), 1);
        assert_eq!(submitted[0].info_hash, dl.info_hash);
    }

    #[test]
    fn delete_download_removes_status_index_entry() {
        let (store, _dir) = new_store();
        let dl = make_download(MAGNET1);
        store.create_download(&dl).unwrap();
        store.delete_download(&dl.info_hash).unwrap();

        let results = store
            .list_downloads_by_status(DownloadStatus::Queued)
            .unwrap();
        assert!(results.is_empty());
    }
}

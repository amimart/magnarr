use redb::{Database, ReadableTable, TableDefinition};

use crate::app::download::{
    DownloadRepository, DownloadsPage, DownloadsPageCursor, RepositoryError,
};
use crate::types::{Download, DownloadStatus};

const DOWNLOADS: TableDefinition<&str, &str> = TableDefinition::new("downloads");
/// Key: `{status}:{info_hash}`, value: info_hash. Enables O(log n) lookup by status.
/// `;` (0x3B) is the next ASCII char after `:` (0x3A); info_hash hex chars are all below it.
const STATUS_INDEX: TableDefinition<&str, &str> = TableDefinition::new("status_index");
/// Key: `{descending_created_at}:{info_hash}`, value: info_hash. Enables ordered pagination.
const CREATED_AT_INDEX: TableDefinition<&str, &str> = TableDefinition::new("created_at_index");

const I64_SIGN_MASK: u64 = 1 << 63;

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

fn normalize_i64(value: i64) -> u64 {
    (value as u64) ^ I64_SIGN_MASK
}

fn created_at_index_key(created_at: chrono::DateTime<chrono::Utc>, info_hash: &str) -> String {
    let sortable_timestamp = normalize_i64(created_at.timestamp_micros());
    let descending_timestamp = u64::MAX - sortable_timestamp;
    format!("{descending_timestamp:016x}:{info_hash}")
}

fn created_at_index_key_after(cursor: &DownloadsPageCursor) -> String {
    format!(
        "{}\0",
        created_at_index_key(cursor.created_at, &cursor.info_hash)
    )
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
            tx.open_table(CREATED_AT_INDEX)
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

            let created_at_key = created_at_index_key(download.created_at, &download.info_hash);
            let mut created_at_idx = tx
                .open_table(CREATED_AT_INDEX)
                .map_err(|e| RepositoryError::Backend(e.to_string()))?;
            created_at_idx
                .insert(created_at_key.as_str(), download.info_hash.as_str())
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

    fn list_downloads_page(
        &self,
        after: Option<&DownloadsPageCursor>,
        limit: usize,
    ) -> Result<DownloadsPage, RepositoryError> {
        if limit == 0 {
            return Ok(DownloadsPage {
                downloads: Vec::new(),
                end_cursor: None,
                has_next_page: false,
            });
        }

        let tx = self
            .db
            .begin_read()
            .map_err(|e| RepositoryError::Backend(e.to_string()))?;

        let hashes: Vec<String> = {
            let idx = tx
                .open_table(CREATED_AT_INDEX)
                .map_err(|e| RepositoryError::Backend(e.to_string()))?;

            let iter = match after {
                Some(cursor) => idx
                    .range(created_at_index_key_after(cursor).as_str()..)
                    .map_err(|e| RepositoryError::Backend(e.to_string()))?,
                None => idx
                    .iter()
                    .map_err(|e| RepositoryError::Backend(e.to_string()))?,
            };

            iter.map(|entry| {
                entry
                    .map(|(_, value)| value.value().to_owned())
                    .map_err(|err: redb::StorageError| RepositoryError::Backend(err.to_string()))
            })
            .take(limit + 1)
            .collect::<Result<Vec<_>, _>>()?
        };

        let has_next_page = hashes.len() > limit;
        let page_hashes = hashes.into_iter().take(limit).collect::<Vec<_>>();

        let table = tx
            .open_table(DOWNLOADS)
            .map_err(|e| RepositoryError::Backend(e.to_string()))?;
        let mut downloads = Vec::with_capacity(page_hashes.len());
        for hash in &page_hashes {
            if let Some(entry) = table
                .get(hash.as_str())
                .map_err(|e| RepositoryError::Backend(e.to_string()))?
            {
                let download: Download = serde_json::from_str(entry.value())
                    .map_err(|e| RepositoryError::Serialization(e.to_string()))?;
                downloads.push(download);
            }
        }

        let end_cursor = downloads.last().map(|download| DownloadsPageCursor {
            created_at: download.created_at,
            info_hash: download.info_hash.clone(),
        });

        Ok(DownloadsPage {
            downloads,
            end_cursor,
            has_next_page,
        })
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
            let old_download = {
                let entry = table
                    .get(download.info_hash.as_str())
                    .map_err(|e| RepositoryError::Backend(e.to_string()))?;
                entry
                    .map(|v| {
                        serde_json::from_str::<Download>(v.value())
                            .map_err(|e| RepositoryError::Serialization(e.to_string()))
                    })
                    .transpose()?
            };

            table
                .insert(download.info_hash.as_str(), json.as_str())
                .map_err(|e| RepositoryError::Backend(e.to_string()))?;

            // Update the status index only when the status actually changed.
            if old_download.as_ref().map(|d| d.status) != Some(download.status) {
                let mut status_idx = tx
                    .open_table(STATUS_INDEX)
                    .map_err(|e| RepositoryError::Backend(e.to_string()))?;
                if let Some(old) = old_download.as_ref() {
                    let old_key = format!("{}:{}", status_prefix(old.status), download.info_hash);
                    status_idx
                        .remove(old_key.as_str())
                        .map_err(|e| RepositoryError::Backend(e.to_string()))?;
                }
                let new_key = format!("{}:{}", status_prefix(download.status), download.info_hash);
                status_idx
                    .insert(new_key.as_str(), download.info_hash.as_str())
                    .map_err(|e| RepositoryError::Backend(e.to_string()))?;
            }

            let old_created_at_key = old_download
                .as_ref()
                .map(|old| created_at_index_key(old.created_at, &old.info_hash));
            let new_created_at_key = created_at_index_key(download.created_at, &download.info_hash);
            if old_created_at_key.as_deref() != Some(new_created_at_key.as_str()) {
                let mut created_at_idx = tx
                    .open_table(CREATED_AT_INDEX)
                    .map_err(|e| RepositoryError::Backend(e.to_string()))?;
                if let Some(old_key) = old_created_at_key {
                    created_at_idx
                        .remove(old_key.as_str())
                        .map_err(|e| RepositoryError::Backend(e.to_string()))?;
                }
                created_at_idx
                    .insert(new_created_at_key.as_str(), download.info_hash.as_str())
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

            let created_at_key = created_at_index_key(download.created_at, &download.info_hash);
            let mut created_at_idx = tx
                .open_table(CREATED_AT_INDEX)
                .map_err(|e| RepositoryError::Backend(e.to_string()))?;
            created_at_idx
                .remove(created_at_key.as_str())
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
    use chrono::{TimeZone, Utc};

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

    #[test]
    fn list_downloads_page_returns_newest_first() {
        let (store, _dir) = new_store();
        let mut oldest = make_download(MAGNET1);
        oldest.created_at = Utc.timestamp_opt(10, 0).unwrap();
        oldest.updated_at = oldest.created_at;

        let mut newest = make_download(MAGNET2);
        newest.created_at = Utc.timestamp_opt(30, 0).unwrap();
        newest.updated_at = newest.created_at;

        let mut middle = make_download(MAGNET3);
        middle.created_at = Utc.timestamp_opt(20, 0).unwrap();
        middle.updated_at = middle.created_at;

        store.create_download(&oldest).unwrap();
        store.create_download(&newest).unwrap();
        store.create_download(&middle).unwrap();

        let page = store.list_downloads_page(None, 10).unwrap();

        assert_eq!(
            page.downloads
                .iter()
                .map(|download| download.info_hash.as_str())
                .collect::<Vec<_>>(),
            vec![
                newest.info_hash.as_str(),
                middle.info_hash.as_str(),
                oldest.info_hash.as_str()
            ]
        );
        assert!(!page.has_next_page);
    }

    #[test]
    fn list_downloads_page_uses_cursor_for_next_page() {
        let (store, _dir) = new_store();
        let mut first = make_download(MAGNET1);
        first.created_at = Utc.timestamp_opt(30, 0).unwrap();
        first.updated_at = first.created_at;

        let mut second = make_download(MAGNET2);
        second.created_at = Utc.timestamp_opt(20, 0).unwrap();
        second.updated_at = second.created_at;

        let mut third = make_download(MAGNET3);
        third.created_at = Utc.timestamp_opt(10, 0).unwrap();
        third.updated_at = third.created_at;

        store.create_download(&first).unwrap();
        store.create_download(&second).unwrap();
        store.create_download(&third).unwrap();

        let first_page = store.list_downloads_page(None, 2).unwrap();
        assert_eq!(first_page.downloads.len(), 2);
        assert!(first_page.has_next_page);

        let second_page = store
            .list_downloads_page(first_page.end_cursor.as_ref(), 2)
            .unwrap();

        assert_eq!(second_page.downloads.len(), 1);
        assert_eq!(second_page.downloads[0].info_hash, third.info_hash);
        assert!(!second_page.has_next_page);
    }

    #[test]
    fn list_downloads_page_is_stable_for_equal_timestamps() {
        let (store, _dir) = new_store();
        let created_at = Utc.timestamp_opt(30, 0).unwrap();

        let mut first = make_download(MAGNET1);
        first.created_at = created_at;
        first.updated_at = created_at;

        let mut second = make_download(MAGNET2);
        second.created_at = created_at;
        second.updated_at = created_at;

        store.create_download(&first).unwrap();
        store.create_download(&second).unwrap();

        let page = store.list_downloads_page(None, 10).unwrap();

        assert_eq!(
            page.downloads
                .iter()
                .map(|download| download.info_hash.as_str())
                .collect::<Vec<_>>(),
            vec![first.info_hash.as_str(), second.info_hash.as_str()]
        );
    }
}

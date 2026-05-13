use redb::{Database, ReadableTable, TableDefinition};
use std::ops::{Bound, RangeBounds};

use crate::app::download::{
    DownloadCursor, SortOrder, DownloadRepository, RepositoryError,
};
use crate::store::iter::{RedbDownloadIndexIter, RedbIndexIter};
use crate::types::{Download, DownloadStatus};

const DOWNLOADS: TableDefinition<&str, &str> = TableDefinition::new("downloads");
/// Key: `{created_at}:{info_hash}`, value: info_hash. Enables ordered iteration across all downloads.
const CREATED_AT_INDEX: TableDefinition<&str, &str> = TableDefinition::new("created_at_index");
/// Key: `{status}:{created_at}:{info_hash}`, value: info_hash. Enables ordered iteration within one status.
/// `;` (0x3B) is the next ASCII char after `:` (0x3A), so `{status};` is an exclusive upper bound.
const STATUS_CREATED_AT_INDEX: TableDefinition<&str, &str> =
    TableDefinition::new("status_created_at_index");

const I64_SIGN_MASK: u64 = 1 << 63;

fn status_prefix(status: DownloadStatus) -> &'static str {
    match status {
        DownloadStatus::Queued => "00",
        DownloadStatus::Submitted => "01",
        DownloadStatus::Downloading => "02",
        DownloadStatus::Completed => "03",
        DownloadStatus::Importing => "04",
        DownloadStatus::Imported => "05",
        DownloadStatus::Failed => "06",
    }
}

fn normalize_i64(value: i64) -> u64 {
    (value as u64) ^ I64_SIGN_MASK
}

fn as_str_bound(bound: Bound<&String>) -> Bound<&str> {
    match bound {
        Bound::Included(value) => Bound::Included(value.as_str()),
        Bound::Excluded(value) => Bound::Excluded(value.as_str()),
        Bound::Unbounded => Bound::Unbounded,
    }
}

fn created_at_index_prefix(created_at: chrono::DateTime<chrono::Utc>) -> String {
    format!("{:016x}", normalize_i64(created_at.timestamp_micros()))
}

fn created_at_index_key(created_at: chrono::DateTime<chrono::Utc>, info_hash: &str) -> String {
    format!("{}:{info_hash}", created_at_index_prefix(created_at))
}

fn status_created_at_index_key(
    status: DownloadStatus,
    created_at: chrono::DateTime<chrono::Utc>,
    info_hash: &str,
) -> String {
    format!(
        "{}:{}",
        status_prefix(status),
        created_at_index_key(created_at, info_hash)
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
                    .map_err(|e| RepositoryError::Storage(e.to_string()))?;
            }
        }

        let db = Database::create(path).map_err(|e| RepositoryError::Storage(e.to_string()))?;

        let tx = db
            .begin_write()
            .map_err(|e| RepositoryError::Storage(e.to_string()))?;
        {
            tx.open_table(DOWNLOADS)
                .map_err(|e| RepositoryError::Storage(e.to_string()))?;
            tx.open_table(CREATED_AT_INDEX)
                .map_err(|e| RepositoryError::Storage(e.to_string()))?;
            tx.open_table(STATUS_CREATED_AT_INDEX)
                .map_err(|e| RepositoryError::Storage(e.to_string()))?;
        }
        tx.commit()
            .map_err(|e| RepositoryError::Storage(e.to_string()))?;

        Ok(Self { db })
    }
}

impl DownloadRepository for RedbStore {
    fn insert(&self, download: &Download) -> Result<(), RepositoryError> {
        let json = serde_json::to_string(download)
            .map_err(|e| RepositoryError::Serde(e.to_string()))?;

        let tx = self
            .db
            .begin_write()
            .map_err(|e| RepositoryError::Storage(e.to_string()))?;
        {
            let mut table = tx
                .open_table(DOWNLOADS)
                .map_err(|e| RepositoryError::Storage(e.to_string()))?;

            if table
                .get(download.info_hash.as_str())
                .map_err(|e| RepositoryError::Storage(e.to_string()))?
                .is_some()
            {
                return Err(RepositoryError::AlreadyExists);
            }

            table
                .insert(download.info_hash.as_str(), json.as_str())
                .map_err(|e| RepositoryError::Storage(e.to_string()))?;

            let created_at_key = created_at_index_key(download.created_at, &download.info_hash);
            let mut created_at_idx = tx
                .open_table(CREATED_AT_INDEX)
                .map_err(|e| RepositoryError::Storage(e.to_string()))?;
            created_at_idx
                .insert(created_at_key.as_str(), download.info_hash.as_str())
                .map_err(|e| RepositoryError::Storage(e.to_string()))?;

            let status_created_at_key = status_created_at_index_key(
                download.status,
                download.created_at,
                &download.info_hash,
            );
            let mut status_created_at_idx = tx
                .open_table(STATUS_CREATED_AT_INDEX)
                .map_err(|e| RepositoryError::Storage(e.to_string()))?;
            status_created_at_idx
                .insert(status_created_at_key.as_str(), download.info_hash.as_str())
                .map_err(|e| RepositoryError::Storage(e.to_string()))?;
        }
        tx.commit()
            .map_err(|e| RepositoryError::Storage(e.to_string()))?;
        Ok(())
    }

    fn get(&self, info_hash: &str) -> Result<Download, RepositoryError> {
        let tx = self
            .db
            .begin_read()
            .map_err(|e| RepositoryError::Storage(e.to_string()))?;
        let table = tx
            .open_table(DOWNLOADS)
            .map_err(|e| RepositoryError::Storage(e.to_string()))?;
        let entry = table
            .get(info_hash)
            .map_err(|e| RepositoryError::Storage(e.to_string()))?
            .ok_or(RepositoryError::NotFound)?;
        serde_json::from_str(entry.value())
            .map_err(|e| RepositoryError::Serde(e.to_string()))
    }

    fn list(
        &self,
        status: Option<DownloadStatus>,
        from: Option<chrono::DateTime<chrono::Utc>>,
        after: Option<DownloadCursor>,
        order: SortOrder,
    ) -> Result<impl Iterator<Item = Result<Download, RepositoryError>>, RepositoryError> {
        let (start_bound, end_bound) = match (status, from) {
            (Some(s), Some(f)) => (
                Bound::Included(format!(
                    "{}:{}:",
                    status_prefix(s),
                    created_at_index_prefix(f)
                )),
                Bound::Excluded(format!("{};", status_prefix(s))),
            ),
            (Some(s), None) => (
                Bound::Included(format!("{}:", status_prefix(s))),
                Bound::Excluded(format!("{};", status_prefix(s))),
            ),
            (None, Some(f)) => (
                Bound::Included(format!("{}:", created_at_index_prefix(f))),
                Bound::Unbounded,
            ),
            (None, None) => (Bound::Unbounded, Bound::Unbounded),
        };

        let (idx_name, cursor_bound) = match status {
            Some(_) => (
                STATUS_CREATED_AT_INDEX,
                after.map(|c| {
                    Bound::Excluded(status_created_at_index_key(
                        c.status,
                        c.created_at,
                        &c.info_hash,
                    ))
                }),
            ),
            None => (
                CREATED_AT_INDEX,
                after.map(|c| Bound::Excluded(created_at_index_key(c.created_at, &c.info_hash))),
            ),
        };

        let range = match cursor_bound {
            None => (start_bound, end_bound),
            Some(cbound) => match order {
                SortOrder::Asc => (cbound, end_bound),
                SortOrder::Desc => (start_bound, cbound),
            },
        };

        let ref_range = (
            as_str_bound(range.start_bound()),
            as_str_bound(range.end_bound()),
        );

        let tx = self
            .db
            .begin_read()
            .map_err(|e| RepositoryError::Storage(e.to_string()))?;

        let idx = tx
            .open_table(idx_name)
            .map_err(|e| RepositoryError::Storage(e.to_string()))?;

        let iter: RedbIndexIter = match order {
            SortOrder::Asc => Box::new(
                idx.range::<&str>(ref_range)
                    .map_err(|e| RepositoryError::Storage(e.to_string()))?,
            ),
            SortOrder::Desc => Box::new(
                idx.range::<&str>(ref_range)
                    .map_err(|e| RepositoryError::Storage(e.to_string()))?
                    .rev(),
            ),
        };

        let downloads = tx
            .open_table(DOWNLOADS)
            .map_err(|e| RepositoryError::Storage(e.to_string()))?;

        Ok(RedbDownloadIndexIter::new(downloads, iter))
    }

    fn update(&self, download: &Download) -> Result<(), RepositoryError> {
        let json = serde_json::to_string(download)
            .map_err(|e| RepositoryError::Serde(e.to_string()))?;

        let tx = self
            .db
            .begin_write()
            .map_err(|e| RepositoryError::Storage(e.to_string()))?;
        {
            let mut table = tx
                .open_table(DOWNLOADS)
                .map_err(|e| RepositoryError::Storage(e.to_string()))?;

            let old_download = {
                let entry = table
                    .get(download.info_hash.as_str())
                    .map_err(|e| RepositoryError::Storage(e.to_string()))?;
                entry
                    .map(|value| {
                        serde_json::from_str::<Download>(value.value())
                            .map_err(|e| RepositoryError::Serde(e.to_string()))
                    })
                    .transpose()?
                    .ok_or(RepositoryError::NotFound)?
            };

            table
                .insert(download.info_hash.as_str(), json.as_str())
                .map_err(|e| RepositoryError::Storage(e.to_string()))?;

            let old_created_at_key =
                created_at_index_key(old_download.created_at, &old_download.info_hash);
            let new_created_at_key = created_at_index_key(download.created_at, &download.info_hash);
            if old_created_at_key != new_created_at_key {
                let mut created_at_idx = tx
                    .open_table(CREATED_AT_INDEX)
                    .map_err(|e| RepositoryError::Storage(e.to_string()))?;
                created_at_idx
                    .remove(old_created_at_key.as_str())
                    .map_err(|e| RepositoryError::Storage(e.to_string()))?;
                created_at_idx
                    .insert(new_created_at_key.as_str(), download.info_hash.as_str())
                    .map_err(|e| RepositoryError::Storage(e.to_string()))?;
            }

            let old_status_created_at_key = status_created_at_index_key(
                old_download.status,
                old_download.created_at,
                &old_download.info_hash,
            );
            let new_status_created_at_key = status_created_at_index_key(
                download.status,
                download.created_at,
                &download.info_hash,
            );
            if old_status_created_at_key != new_status_created_at_key.as_str() {
                let mut status_created_at_idx = tx
                    .open_table(STATUS_CREATED_AT_INDEX)
                    .map_err(|e| RepositoryError::Storage(e.to_string()))?;
                status_created_at_idx
                    .remove(old_status_created_at_key.as_str())
                    .map_err(|e| RepositoryError::Storage(e.to_string()))?;
                status_created_at_idx
                    .insert(
                        new_status_created_at_key.as_str(),
                        download.info_hash.as_str(),
                    )
                    .map_err(|e| RepositoryError::Storage(e.to_string()))?;
            }
        }
        tx.commit()
            .map_err(|e| RepositoryError::Storage(e.to_string()))?;
        Ok(())
    }

    fn remove(&self, info_hash: &str) -> Result<(), RepositoryError> {
        let tx = self
            .db
            .begin_write()
            .map_err(|e| RepositoryError::Storage(e.to_string()))?;
        {
            let mut table = tx
                .open_table(DOWNLOADS)
                .map_err(|e| RepositoryError::Storage(e.to_string()))?;

            let entry = table
                .get(info_hash)
                .map_err(|e| RepositoryError::Storage(e.to_string()))?
                .ok_or(RepositoryError::NotFound)?;

            let download: Download = serde_json::from_str(entry.value())
                .map_err(|e| RepositoryError::Serde(e.to_string()))?;

            drop(entry);

            table
                .remove(info_hash)
                .map_err(|e| RepositoryError::Storage(e.to_string()))?;

            let created_at_key = created_at_index_key(download.created_at, &download.info_hash);
            let mut created_at_idx = tx
                .open_table(CREATED_AT_INDEX)
                .map_err(|e| RepositoryError::Storage(e.to_string()))?;
            created_at_idx
                .remove(created_at_key.as_str())
                .map_err(|e| RepositoryError::Storage(e.to_string()))?;

            let status_created_at_key = status_created_at_index_key(
                download.status,
                download.created_at,
                &download.info_hash,
            );
            let mut status_created_at_idx = tx
                .open_table(STATUS_CREATED_AT_INDEX)
                .map_err(|e| RepositoryError::Storage(e.to_string()))?;
            status_created_at_idx
                .remove(status_created_at_key.as_str())
                .map_err(|e| RepositoryError::Storage(e.to_string()))?;
        }
        tx.commit()
            .map_err(|e| RepositoryError::Storage(e.to_string()))?;
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

    fn collect_downloads(
        iter: impl Iterator<Item = Result<Download, RepositoryError>>,
    ) -> Vec<Download> {
        iter.collect::<Result<Vec<_>, _>>().unwrap()
    }

    #[test]
    fn create_then_find_returns_same_download() {
        let (store, _dir) = new_store();
        let dl = make_download(MAGNET1);
        store.insert(&dl).unwrap();

        let fetched = store.get(&dl.info_hash).unwrap();
        assert_eq!(fetched.info_hash, dl.info_hash);
        assert_eq!(fetched.magnet, dl.magnet);
        assert_eq!(fetched.status, dl.status);
    }

    #[test]
    fn create_twice_same_info_hash_returns_already_exists() {
        let (store, _dir) = new_store();
        let dl = make_download(MAGNET1);
        store.insert(&dl).unwrap();
        let result = store.insert(&dl);
        assert!(matches!(result, Err(RepositoryError::AlreadyExists)));
    }

    #[test]
    fn find_unknown_info_hash_returns_not_found() {
        let (store, _dir) = new_store();
        let result = store.get("0000000000000000000000000000000000000000");
        assert!(matches!(result, Err(RepositoryError::NotFound)));
    }

    #[test]
    fn list_downloads_returns_newest_first_by_default() {
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

        store.insert(&oldest).unwrap();
        store.insert(&newest).unwrap();
        store.insert(&middle).unwrap();

        let downloads = collect_downloads(
            store
                .list(None, None, None, SortOrder::Desc)
                .unwrap(),
        );

        assert_eq!(
            downloads
                .iter()
                .map(|download| download.info_hash.as_str())
                .collect::<Vec<_>>(),
            vec![
                newest.info_hash.as_str(),
                middle.info_hash.as_str(),
                oldest.info_hash.as_str()
            ]
        );
    }

    #[test]
    fn list_downloads_supports_status_filter_with_ordering() {
        let (store, _dir) = new_store();
        let mut queued = make_download(MAGNET1);
        queued.created_at = Utc.timestamp_opt(10, 0).unwrap();
        queued.updated_at = queued.created_at;

        let mut submitted = make_download(MAGNET2);
        submitted.status = DownloadStatus::Submitted;
        submitted.created_at = Utc.timestamp_opt(30, 0).unwrap();
        submitted.updated_at = submitted.created_at;

        let mut submitted_older = make_download(MAGNET3);
        submitted_older.status = DownloadStatus::Submitted;
        submitted_older.created_at = Utc.timestamp_opt(20, 0).unwrap();
        submitted_older.updated_at = submitted_older.created_at;

        store.insert(&queued).unwrap();
        store.insert(&submitted).unwrap();
        store.insert(&submitted_older).unwrap();

        let downloads = collect_downloads(
            store
                .list(
                    Some(DownloadStatus::Submitted),
                    None,
                    None,
                    SortOrder::Desc,
                )
                .unwrap(),
        );

        assert_eq!(downloads.len(), 2);
        assert_eq!(downloads[0].info_hash, submitted.info_hash);
        assert_eq!(downloads[1].info_hash, submitted_older.info_hash);
    }

    #[test]
    fn list_downloads_supports_ascending_order() {
        let (store, _dir) = new_store();
        let mut first = make_download(MAGNET1);
        first.created_at = Utc.timestamp_opt(10, 0).unwrap();
        first.updated_at = first.created_at;

        let mut second = make_download(MAGNET2);
        second.created_at = Utc.timestamp_opt(20, 0).unwrap();
        second.updated_at = second.created_at;

        store.insert(&second).unwrap();
        store.insert(&first).unwrap();

        let downloads = collect_downloads(
            store
                .list(None, None, None, SortOrder::Asc)
                .unwrap(),
        );

        assert_eq!(downloads[0].info_hash, first.info_hash);
        assert_eq!(downloads[1].info_hash, second.info_hash);
    }

    #[test]
    fn list_downloads_supports_created_at_and_after_info_hash_cursor() {
        let (store, _dir) = new_store();
        let created_at = Utc.timestamp_opt(30, 0).unwrap();

        let mut newest = make_download(MAGNET1);
        newest.created_at = created_at;
        newest.updated_at = created_at;

        let mut same_timestamp = make_download(MAGNET2);
        same_timestamp.created_at = created_at;
        same_timestamp.updated_at = created_at;

        let mut older = make_download(MAGNET3);
        older.created_at = Utc.timestamp_opt(20, 0).unwrap();
        older.updated_at = older.created_at;

        store.insert(&newest).unwrap();
        store.insert(&same_timestamp).unwrap();
        store.insert(&older).unwrap();

        let first_page = collect_downloads(
            store
                .list(None, None, None, SortOrder::Desc)
                .unwrap(),
        );
        let second_cursor = &first_page[1];

        let downloads = collect_downloads(
            store
                .list(
                    None,
                    None,
                    Some(DownloadCursor::from_download(second_cursor)),
                    SortOrder::Desc,
                )
                .unwrap(),
        );

        assert_eq!(downloads.len(), 1);
        assert_eq!(downloads[0].info_hash, older.info_hash);
    }

    #[test]
    fn update_download_persists_changes() {
        let (store, _dir) = new_store();
        let dl = make_download(MAGNET1);
        store.insert(&dl).unwrap();

        let mut updated = dl.clone();
        updated.status = DownloadStatus::Downloading;
        updated.touch();
        store.update(&updated).unwrap();

        let fetched = store.get(&dl.info_hash).unwrap();
        assert_eq!(fetched.status, DownloadStatus::Downloading);
        assert!(fetched.updated_at >= dl.updated_at);

        let filtered = collect_downloads(
            store
                .list(
                    Some(DownloadStatus::Downloading),
                    None,
                    None,
                    SortOrder::Desc,
                )
                .unwrap(),
        );
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].info_hash, dl.info_hash);
    }

    #[test]
    fn delete_download_removes_it_from_indexes() {
        let (store, _dir) = new_store();
        let dl = make_download(MAGNET1);
        store.insert(&dl).unwrap();
        store.remove(&dl.info_hash).unwrap();

        assert!(matches!(
            store.get(&dl.info_hash),
            Err(RepositoryError::NotFound)
        ));
        assert!(collect_downloads(
            store
                .list(None, None, None, SortOrder::Desc)
                .unwrap(),
        )
        .is_empty());
        assert!(collect_downloads(
            store
                .list(
                    Some(DownloadStatus::Queued),
                    None,
                    None,
                    SortOrder::Desc,
                )
                .unwrap(),
        )
        .is_empty());
    }
}

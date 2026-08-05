use crate::app::repository::{
    DownloadCursor, DownloadEntry, DownloadRepository, RepositoryError, SortOrder,
};
use crate::types::{Download, DownloadStatus};
use collette::backend::redb::{RedbMultiStore, RedbReadStore};
use collette::index_registry::{Cons, Nil};
use collette::iter::IndexIterator;
use collette::{
    impl_enum_key, Collection, Cursor, Direction, Error, Index, Item, Multi, PrefixableScan, Scan,
};
use std::path::PathBuf;

impl Item for Download {
    type Key<'a>
        = &'a str
    where
        Self: 'a;

    type Error = serde_json::Error;

    fn key(&self) -> Self::Key<'_> {
        self.info_hash.as_str()
    }

    fn to_bytes(&self) -> Result<Vec<u8>, Self::Error> {
        serde_json::to_vec(self)
    }

    fn from_bytes(bytes: &[u8]) -> Result<Self, Self::Error> {
        serde_json::from_slice(bytes)
    }
}

impl_enum_key!(DownloadStatus as u8 {
    DownloadStatus::Queued => 0,
    DownloadStatus::Submitted => 1,
    DownloadStatus::Downloading => 2,
    DownloadStatus::Importing => 3,
    DownloadStatus::Imported => 4,
    DownloadStatus::Failed => 5,
});

impl From<SortOrder> for Direction {
    fn from(order: SortOrder) -> Self {
        match order {
            SortOrder::Asc => Direction::LeftToRight,
            SortOrder::Desc => Direction::RightToLeft,
        }
    }
}

struct CreatedAt;

impl Index<Download> for CreatedAt {
    type Key<'a>
        = (i64,)
    where
        Download: 'a;

    type Kind<'a>
        = Multi
    where
        Download: 'a;

    const NAME: &'static str = "created_at";

    fn key(entity: &Download) -> Self::Key<'_> {
        (entity.created_at.timestamp_micros(),)
    }
}

struct StatusAndCreatedAt;

impl Index<Download> for StatusAndCreatedAt {
    type Key<'a>
        = (DownloadStatus, i64)
    where
        Download: 'a;

    type Kind<'a>
        = Multi
    where
        Download: 'a;

    const NAME: &'static str = "status_and_created_at";

    fn key(entity: &Download) -> Self::Key<'_> {
        (entity.status, entity.created_at.timestamp_micros())
    }
}

pub struct DownloadStore {
    db: Collection<RedbMultiStore, Download, Cons<StatusAndCreatedAt, Cons<CreatedAt, Nil>>>,
}

pub struct DownloadScanIter {
    inner: IndexIterator<RedbReadStore, Download>,
}

impl Iterator for DownloadScanIter {
    type Item = Result<DownloadEntry, RepositoryError>;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().map(|result| {
            result.map_err(RepositoryError::from).and_then(|entry| {
                let cursor = match entry.key {
                    Cursor::Key(key) => DownloadCursor::new(key.to_vec()),
                    Cursor::None => {
                        return Err(RepositoryError::Storage(
                            "missing cursor on download entry".into(),
                        ));
                    }
                };
                Ok(DownloadEntry {
                    download: entry.record,
                    cursor,
                })
            })
        })
    }
}

impl DownloadStore {
    pub fn new(path: PathBuf) -> Result<Self, RepositoryError> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| RepositoryError::Storage(e.to_string()))?;
            }
        }

        let db =
            RedbMultiStore::create(path).map_err(|e| RepositoryError::Storage(e.to_string()))?;

        Ok(Self {
            db: collette::collection::<Download, _>("downloads", db)
                .with_index::<CreatedAt>()
                .with_index::<StatusAndCreatedAt>()
                .build(),
        })
    }
}

impl From<Error> for RepositoryError {
    fn from(value: Error) -> Self {
        match value {
            Error::NotFound(_) => RepositoryError::NotFound,
            Error::AlreadyExists(_) => RepositoryError::AlreadyExists,
            Error::Unexpected(s) => RepositoryError::Storage(s),
            Error::Backend(e) => RepositoryError::Storage(e.to_string()),
            Error::Codec(e) => RepositoryError::Serde(e.to_string()),
            Error::CursorOutOfBounds => RepositoryError::Storage("cursor out of bounds".into()),
        }
    }
}

impl DownloadRepository for DownloadStore {
    type Iter<'a> = DownloadScanIter;

    fn insert(&self, download: &Download) -> Result<(), RepositoryError> {
        self.db.insert(download).map_err(RepositoryError::from)
    }

    fn get(&self, info_hash: &str) -> Result<Download, RepositoryError> {
        self.db
            .get(info_hash.to_owned())
            .map_err(RepositoryError::from)
            .and_then(|res| match res {
                Some(download) => Ok(download),
                None => Err(RepositoryError::NotFound),
            })
    }

    fn scan_all(
        &self,
        after: Option<DownloadCursor>,
        order: SortOrder,
    ) -> Result<Self::Iter<'_>, RepositoryError> {
        let inner = self
            .db
            .index_scan(CreatedAt)?
            .direction(order.into())
            .after(to_collette_cursor(after))
            .iter()?;
        Ok(DownloadScanIter { inner })
    }

    fn scan_by_status(
        &self,
        status: DownloadStatus,
        after: Option<DownloadCursor>,
        order: SortOrder,
    ) -> Result<Self::Iter<'_>, RepositoryError> {
        let inner = self
            .db
            .index_scan(StatusAndCreatedAt)
            .map_err(|e| RepositoryError::Storage(e.to_string()))?
            .prefix(status)
            .direction(order.into())
            .after(to_collette_cursor(after))
            .iter()?;
        Ok(DownloadScanIter { inner })
    }

    fn scan_since(
        &self,
        since: chrono::DateTime<chrono::Utc>,
        after: Option<DownloadCursor>,
        order: SortOrder,
    ) -> Result<Self::Iter<'_>, RepositoryError> {
        let inner = self
            .db
            .index_scan(CreatedAt)?
            .range((since.timestamp_micros(),)..)
            .direction(order.into())
            .after(to_collette_cursor(after))
            .iter()?;
        Ok(DownloadScanIter { inner })
    }

    fn scan_by_status_since(
        &self,
        status: DownloadStatus,
        since: chrono::DateTime<chrono::Utc>,
        after: Option<DownloadCursor>,
        order: SortOrder,
    ) -> Result<Self::Iter<'_>, RepositoryError> {
        let inner = self
            .db
            .index_scan(StatusAndCreatedAt)
            .map_err(|e| RepositoryError::Storage(e.to_string()))?
            .prefix(status)
            .range(since.timestamp_micros()..)
            .direction(order.into())
            .after(to_collette_cursor(after))
            .iter()?;
        Ok(DownloadScanIter { inner })
    }

    fn update(&self, download: &Download) -> Result<(), RepositoryError> {
        self.db.update(download).map_err(RepositoryError::from)
    }

    fn remove(&self, info_hash: &str) -> Result<(), RepositoryError> {
        self.db
            .remove(info_hash.to_owned())
            .map_err(RepositoryError::from)
    }
}

fn to_collette_cursor(cursor: Option<DownloadCursor>) -> Cursor {
    cursor
        .map(|cursor| Cursor::Key(cursor.as_ref().to_vec().into()))
        .unwrap_or(Cursor::None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    const MAGNET1: &str = "magnet:?xt=urn:btih:ABCDEF1234567890ABCDEF1234567890ABCDEF12&dn=test1";
    const MAGNET2: &str = "magnet:?xt=urn:btih:FEDCBA0987654321FEDCBA0987654321FEDCBA09&dn=test2";
    const MAGNET3: &str = "magnet:?xt=urn:btih:1111111111111111111111111111111111111111&dn=test3";

    fn new_store() -> (DownloadStore, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.redb");
        let store = DownloadStore::new(path).unwrap();
        (store, dir)
    }

    fn make_download(magnet_str: &str) -> Download {
        let magnet = magnet_str.parse().unwrap();
        Download::new(magnet, "/downloads".to_owned())
    }

    fn make_download_at(magnet_str: &str, status: DownloadStatus, timestamp: i64) -> Download {
        let mut download = make_download(magnet_str);
        download.status = status;
        download.created_at = Utc.timestamp_opt(timestamp, 0).unwrap();
        download.updated_at = download.created_at;
        download
    }

    fn collect_downloads(
        iter: impl Iterator<Item = Result<DownloadEntry, RepositoryError>>,
    ) -> Vec<Download> {
        iter.map(|entry| entry.map(|entry| entry.download))
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
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
    fn scans_return_empty_iterators_for_an_empty_store() {
        let (store, _dir) = new_store();
        let since = Utc.timestamp_opt(10, 0).unwrap();

        assert!(collect_downloads(store.scan_all(None, SortOrder::Asc).unwrap()).is_empty());
        assert!(collect_downloads(
            store
                .scan_by_status(DownloadStatus::Queued, None, SortOrder::Asc)
                .unwrap(),
        )
        .is_empty());
        assert!(
            collect_downloads(store.scan_since(since, None, SortOrder::Asc).unwrap()).is_empty()
        );
        assert!(collect_downloads(
            store
                .scan_by_status_since(DownloadStatus::Queued, since, None, SortOrder::Asc,)
                .unwrap(),
        )
        .is_empty());
    }

    #[test]
    fn scan_all_returns_newest_first_by_default() {
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

        let downloads = collect_downloads(store.scan_all(None, SortOrder::Desc).unwrap());

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
    fn scan_by_status_filters_with_ordering() {
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
                .scan_by_status(DownloadStatus::Submitted, None, SortOrder::Desc)
                .unwrap(),
        );

        assert_eq!(downloads.len(), 2);
        assert_eq!(downloads[0].info_hash, submitted.info_hash);
        assert_eq!(downloads[1].info_hash, submitted_older.info_hash);
    }

    #[test]
    fn scan_by_status_supports_cursor_pagination() {
        let (store, _dir) = new_store();
        let first = make_download_at(MAGNET1, DownloadStatus::Submitted, 10);
        let second = make_download_at(MAGNET2, DownloadStatus::Submitted, 20);
        let excluded = make_download_at(MAGNET3, DownloadStatus::Queued, 30);

        store.insert(&first).unwrap();
        store.insert(&second).unwrap();
        store.insert(&excluded).unwrap();

        let first_page = store
            .scan_by_status(DownloadStatus::Submitted, None, SortOrder::Asc)
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        let downloads = collect_downloads(
            store
                .scan_by_status(
                    DownloadStatus::Submitted,
                    Some(first_page[0].cursor.clone()),
                    SortOrder::Asc,
                )
                .unwrap(),
        );

        assert_eq!(downloads.len(), 1);
        assert_eq!(downloads[0].info_hash, second.info_hash);
    }

    #[test]
    fn scan_all_supports_ascending_order() {
        let (store, _dir) = new_store();
        let mut first = make_download(MAGNET1);
        first.created_at = Utc.timestamp_opt(10, 0).unwrap();
        first.updated_at = first.created_at;

        let mut second = make_download(MAGNET2);
        second.created_at = Utc.timestamp_opt(20, 0).unwrap();
        second.updated_at = second.created_at;

        store.insert(&second).unwrap();
        store.insert(&first).unwrap();

        let downloads = collect_downloads(store.scan_all(None, SortOrder::Asc).unwrap());

        assert_eq!(downloads[0].info_hash, first.info_hash);
        assert_eq!(downloads[1].info_hash, second.info_hash);
    }

    #[test]
    fn scan_all_supports_cursor_pagination() {
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

        let first_page = store
            .scan_all(None, SortOrder::Desc)
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        let second_cursor = first_page[1].cursor.clone();

        let downloads = collect_downloads(
            store
                .scan_all(Some(second_cursor), SortOrder::Desc)
                .unwrap(),
        );

        assert_eq!(downloads.len(), 1);
        assert_eq!(downloads[0].info_hash, older.info_hash);
    }

    #[test]
    fn scan_since_and_scan_by_status_since_apply_creation_lower_bound() {
        let (store, _dir) = new_store();

        let mut old_submitted = make_download(MAGNET1);
        old_submitted.status = DownloadStatus::Submitted;
        old_submitted.created_at = Utc.timestamp_opt(10, 0).unwrap();
        old_submitted.updated_at = old_submitted.created_at;

        let mut recent_queued = make_download(MAGNET2);
        recent_queued.created_at = Utc.timestamp_opt(20, 0).unwrap();
        recent_queued.updated_at = recent_queued.created_at;

        let mut recent_submitted = make_download(MAGNET3);
        recent_submitted.status = DownloadStatus::Submitted;
        recent_submitted.created_at = Utc.timestamp_opt(30, 0).unwrap();
        recent_submitted.updated_at = recent_submitted.created_at;

        store.insert(&old_submitted).unwrap();
        store.insert(&recent_queued).unwrap();
        store.insert(&recent_submitted).unwrap();

        let since = Utc.timestamp_opt(20, 0).unwrap();
        let recent = collect_downloads(store.scan_since(since, None, SortOrder::Asc).unwrap());
        let recent_submitted_downloads = collect_downloads(
            store
                .scan_by_status_since(DownloadStatus::Submitted, since, None, SortOrder::Asc)
                .unwrap(),
        );

        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].info_hash, recent_queued.info_hash);
        assert_eq!(recent[1].info_hash, recent_submitted.info_hash);
        assert_eq!(recent_submitted_downloads.len(), 1);
        assert_eq!(
            recent_submitted_downloads[0].info_hash,
            recent_submitted.info_hash
        );
    }

    #[test]
    fn since_scans_are_inclusive_and_support_descending_cursor_pagination() {
        let (store, _dir) = new_store();
        let excluded = make_download_at(MAGNET1, DownloadStatus::Submitted, 10);
        let boundary = make_download_at(MAGNET2, DownloadStatus::Submitted, 20);
        let newest = make_download_at(MAGNET3, DownloadStatus::Submitted, 30);

        store.insert(&excluded).unwrap();
        store.insert(&boundary).unwrap();
        store.insert(&newest).unwrap();

        let since = Utc.timestamp_opt(20, 0).unwrap();
        let first_page = store
            .scan_since(since, None, SortOrder::Desc)
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        let status_first_page = store
            .scan_by_status_since(DownloadStatus::Submitted, since, None, SortOrder::Desc)
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        let remaining = collect_downloads(
            store
                .scan_since(since, Some(first_page[0].cursor.clone()), SortOrder::Desc)
                .unwrap(),
        );
        let status_remaining = collect_downloads(
            store
                .scan_by_status_since(
                    DownloadStatus::Submitted,
                    since,
                    Some(status_first_page[0].cursor.clone()),
                    SortOrder::Desc,
                )
                .unwrap(),
        );

        assert_eq!(first_page.len(), 2);
        assert_eq!(first_page[0].download.info_hash, newest.info_hash);
        assert_eq!(first_page[1].download.info_hash, boundary.info_hash);
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].info_hash, boundary.info_hash);
        assert_eq!(status_remaining.len(), 1);
        assert_eq!(status_remaining[0].info_hash, boundary.info_hash);
    }

    #[test]
    fn cursor_from_another_scan_is_rejected() {
        let (store, _dir) = new_store();
        let download = make_download_at(MAGNET1, DownloadStatus::Queued, 10);
        store.insert(&download).unwrap();

        let cursor = store
            .scan_all(None, SortOrder::Asc)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .cursor;
        let result = store
            .scan_by_status(DownloadStatus::Queued, Some(cursor), SortOrder::Asc)
            .and_then(|iter| iter.collect::<Result<Vec<_>, _>>().map(|_| ()));

        assert!(matches!(result, Err(RepositoryError::Storage(_))));
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
                .scan_by_status(DownloadStatus::Downloading, None, SortOrder::Desc)
                .unwrap(),
        );
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].info_hash, dl.info_hash);
        assert!(collect_downloads(
            store
                .scan_by_status(DownloadStatus::Queued, None, SortOrder::Desc)
                .unwrap(),
        )
        .is_empty());
    }

    #[test]
    fn downloads_persist_when_the_store_is_reopened() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/data/magnarr.redb");
        let download = make_download(MAGNET1);

        {
            let store = DownloadStore::new(path.clone()).unwrap();
            store.insert(&download).unwrap();
        }

        let reopened = DownloadStore::new(path).unwrap();
        let fetched = reopened.get(&download.info_hash).unwrap();

        assert_eq!(fetched, download);
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
        assert!(collect_downloads(store.scan_all(None, SortOrder::Desc).unwrap(),).is_empty());
        assert!(collect_downloads(
            store
                .scan_by_status(DownloadStatus::Queued, None, SortOrder::Desc)
                .unwrap(),
        )
        .is_empty());
    }
}

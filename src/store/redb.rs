use crate::app::download::{DownloadRepository, RepositoryError, SortOrder};
use crate::types::{Download, DownloadStatus};
use collette::backend::redb::{RedbMultiStore, RedbReadStore};
use collette::index_registry::{Cons, Nil};
use collette::iter::{Entry, IndexIterator};
use collette::{
    impl_enum_key, Collection, Cursor, Error, Index, Item, Multi, PrefixableScan, Scan,
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

pub struct RedbStore {
    db: Collection<RedbMultiStore, Download, Cons<StatusAndCreatedAt, Cons<CreatedAt, Nil>>>,
}

pub struct RedbDownloadIter {
    inner: IndexIterator<RedbReadStore, Download>,
}

impl Iterator for RedbDownloadIter {
    type Item = Result<Entry<Download>, RepositoryError>;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner
            .next()
            .map(|result| result.map_err(RepositoryError::from))
    }
}

impl RedbStore {
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

impl DownloadRepository for RedbStore {
    type Iter<'a> = RedbDownloadIter;

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

    fn list(
        &self,
        status: Option<DownloadStatus>,
        from: Option<chrono::DateTime<chrono::Utc>>,
        after: Cursor,
        order: SortOrder,
    ) -> Result<Self::Iter<'_>, RepositoryError> {
        let inner = match (status, from) {
            (Some(status), Some(from)) => self
                .db
                .index_scan(StatusAndCreatedAt)
                .map_err(|e| RepositoryError::Storage(e.to_string()))?
                .prefix(status)
                .range(from.timestamp_micros()..)
                .direction(order.into())
                .after(after)
                .iter()?,
            (Some(status), None) => self
                .db
                .index_scan(StatusAndCreatedAt)
                .map_err(|e| RepositoryError::Storage(e.to_string()))?
                .prefix(status)
                .direction(order.into())
                .after(after)
                .iter()?,
            (None, Some(from)) => self
                .db
                .index_scan(CreatedAt)?
                .range((from.timestamp_micros(),)..)
                .direction(order.into())
                .after(after)
                .iter()?,
            (None, None) => self
                .db
                .index_scan(CreatedAt)?
                .direction(order.into())
                .after(after)
                .iter()?,
        };
        Ok(RedbDownloadIter { inner })
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
        let store = RedbStore::new(path).unwrap();
        (store, dir)
    }

    fn make_download(magnet_str: &str) -> Download {
        let magnet = magnet_str.parse().unwrap();
        Download::new(magnet, "/downloads".to_owned())
    }

    fn collect_downloads(
        iter: impl Iterator<Item = Result<Entry<Download>, RepositoryError>>,
    ) -> Vec<Download> {
        iter.map(|entry| entry.map(|entry| entry.record))
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
                .list(None, None, Cursor::None, SortOrder::Desc)
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
                    Cursor::None,
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
                .list(None, None, Cursor::None, SortOrder::Asc)
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

        let first_page = store
            .list(None, None, Cursor::None, SortOrder::Desc)
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        let second_cursor = first_page[1].key.clone();

        let downloads = collect_downloads(
            store
                .list(None, None, second_cursor, SortOrder::Desc)
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
                    Cursor::None,
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
                .list(None, None, Cursor::None, SortOrder::Desc)
                .unwrap(),
        )
        .is_empty());
        assert!(collect_downloads(
            store
                .list(
                    Some(DownloadStatus::Queued),
                    None,
                    Cursor::None,
                    SortOrder::Desc,
                )
                .unwrap(),
        )
        .is_empty());
    }
}

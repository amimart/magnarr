use crate::app::download::RepositoryError;
use crate::types::Download;
use redb::{AccessGuard, ReadOnlyTable};

pub struct RedbDownloadIndexIter {
    downloads: ReadOnlyTable<&'static str, &'static str>,
    index: RedbIndexIter,
}

pub type RedbIndexEntry = redb::Result<(
    AccessGuard<'static, &'static str>,
    AccessGuard<'static, &'static str>,
)>;

pub type RedbIndexIter = Box<dyn Iterator<Item = RedbIndexEntry>>;

impl RedbDownloadIndexIter {
    pub fn new(downloads: ReadOnlyTable<&'static str, &'static str>, index: RedbIndexIter) -> Self {
        Self { downloads, index }
    }
}

impl Iterator for RedbDownloadIndexIter {
    type Item = Result<Download, RepositoryError>;

    fn next(&mut self) -> Option<Self::Item> {
        let next_entry = self.index.next()?;

        Some(match next_entry {
            Ok((_, info_hash)) => {
                let info_hash = info_hash.value();
                match self
                    .downloads
                    .get(info_hash)
                    .map_err(|e| RepositoryError::Backend(e.to_string()))
                {
                    Ok(Some(download)) => serde_json::from_str(download.value())
                        .map_err(|e| RepositoryError::Serialization(e.to_string())),
                    Ok(None) => Err(RepositoryError::Backend(format!(
                        "dangling download index entry for {info_hash}"
                    ))),
                    Err(e) => Err(e),
                }
            }
            Err(e) => Err(RepositoryError::Backend(e.to_string())),
        })
    }
}

use crate::app::download::RepositoryError;
use crate::types::Download;
use collette::backend::redb::RedbReadStore;
use collette::iter::IndexIterator;

pub struct DownloadIter {
    inner: IndexIterator<RedbReadStore, Download>,
}

impl DownloadIter {
    pub fn new(inner: IndexIterator<RedbReadStore, Download>) -> Self {
        Self { inner }
    }
}

impl Iterator for DownloadIter {
    type Item = Result<Download, RepositoryError>;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next()
            .map(|res| match res {
                Ok(entry) => Ok(entry.record),
                Err(err) => Err(err.into()),
            })
    }
}

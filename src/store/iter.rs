use collette::backend::redb::RedbReadStore;
use collette::iter::{CollectionIterator, Entry, IndexIterator};
use crate::app::download::RepositoryError;
use crate::types::Download;

pub enum DownloadIter {
    IndexScan(IndexIterator<RedbReadStore, Download>),
    ColScan(CollectionIterator<RedbReadStore, Download>),
}

impl Iterator for DownloadIter {
    type Item = Result<Entry<Download>, RepositoryError>;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            DownloadIter::IndexScan(iter) => iter.next(),
            DownloadIter::ColScan(iter) => iter.next(),
        }.map(|res| res.map_err(Into::into))
    }
}

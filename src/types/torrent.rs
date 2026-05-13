#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TorrentState {
    Downloading,
    Seeding,
    Paused,
    Error,
    Unknown,
}

#[derive(Debug, Clone)]
pub struct TorrentStatus {
    /// Hash of the torrent.
    pub hash: String,
    /// Current state of the torrent.
    pub state: TorrentState,
    /// Torrent name.
    pub name: String,
    /// Torrent filename or root folder name.
    pub content_name: String,
}

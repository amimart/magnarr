pub mod download;
pub mod model;
pub mod torrent;

use std::sync::Arc;
use std::time::Duration;

use crate::app::download::DownloadRepository;

pub struct App {
    repository: Arc<dyn DownloadRepository>,
    torrent_client: Arc<dyn TorrentClient>,
}

impl App {
    pub fn new(
        repository: Arc<dyn DownloadRepository>,
        torrent_client: Arc<dyn TorrentClient>,
    ) -> Self {
        Self {
            repository,
            torrent_client,
        }
    }
}
    }
}


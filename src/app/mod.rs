pub mod download;
pub mod model;

use crate::app::download::DownloadRepository;

pub struct App {
    #[allow(dead_code)]
    repository: Box<dyn DownloadRepository>,
}

impl App {
    pub fn new(repository: Box<dyn DownloadRepository>) -> Self {
        Self { repository }
    }
}

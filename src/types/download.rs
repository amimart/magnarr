use crate::types::Magnet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DownloadStatus {
    Queued,
    Submitted,
    Downloading,
    Completed,
    Importing,
    Imported,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Download {
    pub info_hash: String,
    pub magnet: Magnet,
    pub name: String,
    pub status: DownloadStatus,
    pub target_dir: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub imported_path: Option<String>,
    pub error: Option<String>,
}

impl Download {
    pub fn touch(&mut self) {
        self.updated_at = chrono::Utc::now();
    }

    pub fn new(magnet: Magnet, target_dir: String) -> Self {
        let now = chrono::Utc::now();
        let info_hash = magnet.info_hash().to_owned();
        let name = magnet.name().unwrap_or("").to_owned();
        Self {
            info_hash,
            magnet,
            name,
            status: DownloadStatus::Queued,
            target_dir,
            created_at: now,
            updated_at: now,
            imported_path: None,
            error: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;


    #[test]
    fn download_new_sets_correct_initial_values() {
        let uri: Magnet = "magnet:?xt=urn:btih:ABCDEF1234567890ABCDEF1234567890ABCDEF12&dn=test".parse().unwrap();
        let dl = Download::new(uri, "/downloads".to_owned());

        assert_eq!(dl.status, DownloadStatus::Queued);
        assert_eq!(dl.name, "test");
        assert_eq!(dl.target_dir, "/downloads");
        assert_eq!(
            dl.info_hash,
            "ABCDEF1234567890ABCDEF1234567890ABCDEF12"
        );
        assert!(dl.imported_path.is_none());
        assert!(dl.error.is_none());
        assert_eq!(dl.created_at, dl.updated_at);
    }
}

use std::str::FromStr;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum ModelError {
    #[error("invalid URI: {0}")]
    InvalidUri(String),
    #[error("invalid scheme: {0}")]
    InvalidScheme(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MagnetUri(String);

impl FromStr for MagnetUri {
    type Err = ModelError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parsed = url::Url::parse(s).map_err(|e| ModelError::InvalidUri(e.to_string()))?;
        if parsed.scheme() != "magnet" {
            return Err(ModelError::InvalidScheme(parsed.scheme().to_owned()));
        }
        Ok(MagnetUri(s.to_owned()))
    }
}

impl MagnetUri {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Extracts the info hash from the `xt=urn:btih:<hash>` query parameter.
    /// Recomputed on each call from the stored string.
    pub fn info_hash(&self) -> Option<&str> {
        let parsed = url::Url::parse(&self.0).ok()?;
        for (key, value) in parsed.query_pairs() {
            if key.eq_ignore_ascii_case("xt") {
                let prefix = "urn:btih:";
                if value.to_lowercase().starts_with(prefix) {
                    let start = self.0.find(value.as_ref())?;
                    return Some(&self.0[start + prefix.len()..start + value.len()]);
                }
            }
        }
        None
    }
}

impl serde::Serialize for MagnetUri {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> serde::Deserialize<'de> for MagnetUri {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

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
    pub id: uuid::Uuid,
    pub magnet_uri: MagnetUri,
    pub info_hash: Option<String>,
    pub torrent_id: Option<String>,
    pub status: DownloadStatus,
    pub target_dir: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub imported_path: Option<String>,
    pub error: Option<String>,
}

impl Download {
    pub fn new(magnet_uri: MagnetUri, target_dir: String) -> Self {
        let now = chrono::Utc::now();
        let info_hash = magnet_uri.info_hash().map(str::to_owned);
        Self {
            id: uuid::Uuid::new_v4(),
            magnet_uri,
            info_hash,
            torrent_id: None,
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

    const MAGNET_WITH_HASH: &str =
        "magnet:?xt=urn:btih:ABCDEF1234567890ABCDEF1234567890ABCDEF12&dn=test";
    const MAGNET_NO_HASH: &str = "magnet:?dn=nodisplay";

    #[test]
    fn valid_magnet_uri_parses_successfully() {
        let result = MAGNET_WITH_HASH.parse::<MagnetUri>();
        assert!(result.is_ok());
    }

    #[test]
    fn info_hash_extracts_correct_hash() {
        let uri: MagnetUri = MAGNET_WITH_HASH.parse().unwrap();
        assert_eq!(
            uri.info_hash(),
            Some("ABCDEF1234567890ABCDEF1234567890ABCDEF12")
        );
    }

    #[test]
    fn magnet_uri_without_xt_returns_none_for_info_hash() {
        let uri: MagnetUri = MAGNET_NO_HASH.parse().unwrap();
        assert_eq!(uri.info_hash(), None);
    }

    #[test]
    fn non_magnet_scheme_returns_model_error() {
        let result = "https://example.com".parse::<MagnetUri>();
        assert!(matches!(result, Err(ModelError::InvalidScheme(_))));
    }

    #[test]
    fn garbage_string_returns_model_error() {
        let result = "not a uri !!@@##".parse::<MagnetUri>();
        assert!(matches!(result, Err(ModelError::InvalidUri(_))));
    }

    #[test]
    fn download_new_sets_correct_initial_values() {
        let uri: MagnetUri = MAGNET_WITH_HASH.parse().unwrap();
        let dl = Download::new(uri, "/downloads".to_owned());

        assert_eq!(dl.status, DownloadStatus::Queued);
        assert_eq!(dl.target_dir, "/downloads");
        assert_eq!(dl.id.get_version(), Some(uuid::Version::Random));
        assert_eq!(
            dl.info_hash.as_deref(),
            Some("ABCDEF1234567890ABCDEF1234567890ABCDEF12")
        );
        assert!(dl.torrent_id.is_none());
        assert!(dl.imported_path.is_none());
        assert!(dl.error.is_none());
        assert_eq!(dl.created_at, dl.updated_at);
    }
}

use std::str::FromStr;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum MagnetParseError {
    #[error("invalid URI: {0}")]
    InvalidUri(String),
    #[error("invalid scheme: {0}")]
    InvalidScheme(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MagnetUri(String);

impl FromStr for MagnetUri {
    type Err = MagnetParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parsed = url::Url::parse(s).map_err(|e| MagnetParseError::InvalidUri(e.to_string()))?;
        if parsed.scheme() != "magnet" {
            return Err(MagnetParseError::InvalidScheme(parsed.scheme().to_owned()));
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
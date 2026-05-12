use std::str::FromStr;
use thiserror::Error;
use url::ParseError;

#[derive(Debug, Error)]
pub enum MagnetParseError {
    #[error("invalid URI: {0}")]
    InvalidUri(ParseError),
    #[error("invalid scheme: {0}")]
    InvalidScheme(String),
    #[error("invalid scheme: {0}")]
    MissingInfoHash(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Magnet {
    raw: String,
    info_hash: String,
    name: Option<String>,
}

impl FromStr for Magnet {
    type Err = MagnetParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parsed = url::Url::parse(s).map_err(|e| MagnetParseError::InvalidUri(e))?;
        if parsed.scheme() != "magnet" {
            return Err(MagnetParseError::InvalidScheme(parsed.scheme().to_owned()));
        }

        let mut hash: Option<String> = None;
        let mut name: Option<String> = None;
        for (key, value) in parsed.query_pairs() {
            if key.eq_ignore_ascii_case("dn") {
                name = Some(value.as_ref().to_owned());
            }

            if key.eq_ignore_ascii_case("xt") {
                let prefix = "urn:btih:";
                if value.to_lowercase().starts_with(prefix) {
                    hash = Some(value.as_ref()[prefix.len()..].to_owned());
                }
            }

            if hash.is_some() && name.is_some() {
                break;
            }
        }

        hash.ok_or_else(|| MagnetParseError::InvalidScheme(parsed.scheme().to_owned()))
            .map(|h| Magnet{
                raw: s.to_owned(),
                info_hash: h,
                name,
            })
    }
}

impl Magnet {
    pub fn as_str(&self) -> &str {
        self.raw.as_str()
    }

    /// Return the info hash from the `xt=urn:btih:<hash>` query parameter.
    pub fn info_hash(&self) -> &str {
        self.info_hash.as_str()
    }

    /// Return the name from the `dn=<name>` query parameter.
    pub fn name(&self) -> Option<&str> {
        self.name.as_ref().map(String::as_str)
    }
}

impl serde::Serialize for Magnet {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.raw)
    }
}

impl<'de> serde::Deserialize<'de> for Magnet {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use crate::types::MagnetParseError;
    use super::*;

    const MAGNET_WITH_HASH: &str =
        "magnet:?xt=urn:btih:ABCDEF1234567890ABCDEF1234567890ABCDEF12&dn=test";
    const MAGNET_WITHOUT_NAME: &str =
        "magnet:?xt=urn:btih:ABCDEF1234567890ABCDEF1234567890ABCDEF12";
    const MAGNET_NO_HASH: &str = "magnet:?dn=nodisplay";

    #[test]
    fn valid_magnet_parses_successfully() {
        let result = MAGNET_WITH_HASH.parse::<Magnet>();
        assert!(result.is_ok());
    }

    #[test]
    fn info_hash_extracts_correct_hash() {
        let uri: Magnet = MAGNET_WITH_HASH.parse().unwrap();
        assert_eq!(
            uri.info_hash(),
            "ABCDEF1234567890ABCDEF1234567890ABCDEF12"
        );
    }

    #[test]
    fn name_extracts_correct_name() {
        let uri: Magnet = MAGNET_WITH_HASH.parse().unwrap();
        assert_eq!(
            uri.name(),
            Some("test"),
        );
    }

    #[test]
    fn magnet_without_name_ok() {
        let uri: Magnet = MAGNET_WITHOUT_NAME.parse().unwrap();
        assert_eq!(
            uri.name(),
            None,
        );
    }

    #[test]
    fn magnet_without_xt_returns_err() {
        let result = MAGNET_NO_HASH.parse::<Magnet>();
        assert!(matches!(result, Err(MagnetParseError::MissingInfoHash(_))));
    }

    #[test]
    fn non_magnet_scheme_returns_err() {
        let result = "https://example.com".parse::<Magnet>();
        assert!(matches!(result, Err(MagnetParseError::InvalidScheme(_))));
    }

    #[test]
    fn garbage_string_returns_err() {
        let result = "not a uri !!@@##".parse::<Magnet>();
        assert!(matches!(result, Err(MagnetParseError::InvalidUri(_))));
    }
}

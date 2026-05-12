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
        let parsed = url::Url::parse(s).map_err(MagnetParseError::InvalidUri)?;
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

        hash.ok_or_else(|| MagnetParseError::MissingInfoHash(s.to_owned()))
            .map(|h| Magnet {
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
        self.name.as_deref()
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
    use super::*;

    struct ParseOk {
        input: &'static str,
        info_hash: &'static str,
        name: Option<&'static str>,
    }

    struct ParseErr {
        input: &'static str,
        error: fn(&MagnetParseError) -> bool,
    }

    #[test]
    fn parse_valid_magnets() {
        let cases = [
            ParseOk {
                input: "magnet:?xt=urn:btih:ABCDEF1234567890ABCDEF1234567890ABCDEF12&dn=test",
                info_hash: "ABCDEF1234567890ABCDEF1234567890ABCDEF12",
                name: Some("test"),
            },
            ParseOk {
                input: "magnet:?xt=urn:btih:ABCDEF1234567890ABCDEF1234567890ABCDEF12",
                info_hash: "ABCDEF1234567890ABCDEF1234567890ABCDEF12",
                name: None,
            },
            ParseOk {
                input: "magnet:?xt=urn:btih:abcdef1234567890abcdef1234567890abcdef12&dn=lower",
                info_hash: "abcdef1234567890abcdef1234567890abcdef12",
                name: Some("lower"),
            },
        ];

        for case in &cases {
            let magnet: Magnet = case
                .input
                .parse()
                .unwrap_or_else(|e| panic!("expected Ok for {:?}, got: {e}", case.input));
            assert_eq!(
                magnet.info_hash(),
                case.info_hash,
                "info_hash mismatch for {:?}",
                case.input
            );
            assert_eq!(
                magnet.name(),
                case.name,
                "name mismatch for {:?}",
                case.input
            );
        }
    }

    #[test]
    fn parse_invalid_magnets() {
        let cases = [
            ParseErr {
                input: "magnet:?dn=nodisplay",
                error: |e| matches!(e, MagnetParseError::MissingInfoHash(_)),
            },
            ParseErr {
                input: "https://example.com",
                error: |e| matches!(e, MagnetParseError::InvalidScheme(_)),
            },
            ParseErr {
                input: "not a uri !!@@##",
                error: |e| matches!(e, MagnetParseError::InvalidUri(_)),
            },
        ];

        for case in &cases {
            let err = case.input.parse::<Magnet>().unwrap_err();
            assert!(
                (case.error)(&err),
                "unexpected error variant for {:?}: {err}",
                case.input
            );
        }
    }
}

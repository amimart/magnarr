use async_trait::async_trait;
use reqwest::Client;
use std::path::PathBuf;
use std::sync::Arc;

use crate::app::torrent::{TorrentClient, TorrentClientError};
use crate::client::http::{AuthenticatedClient, AuthenticatedClientError, LoginFn};
use crate::types::{Magnet, TorrentState, TorrentStatus};

#[derive(Debug, Clone)]
pub struct QbittorrentConfig {
    pub host: String,
    pub username: String,
    pub password: String,
}

pub struct QbittorrentClient {
    host: String,
    http: AuthenticatedClient,
}

impl From<AuthenticatedClientError> for TorrentClientError {
    fn from(value: AuthenticatedClientError) -> Self {
        match value {
            AuthenticatedClientError::AuthFailed(s) => TorrentClientError::AuthFailed(s),
            AuthenticatedClientError::ClientError(e) => TorrentClientError::ClientError(e),
        }
    }
}

impl QbittorrentClient {
    pub fn new(cfg: QbittorrentConfig) -> Self {
        Self {
            host: cfg.host.clone(),
            http: AuthenticatedClient::new(
                Client::builder().cookie_store(true).build().unwrap(),
                Self::make_login_fn(cfg),
            ),
        }
    }

    fn make_login_fn(cfg: QbittorrentConfig) -> Arc<LoginFn> {
        Arc::new(move |client: Client| {
            let url = format!("{}/api/v2/auth/login", cfg.host);
            let username = cfg.username.clone();
            let password = cfg.password.clone();

            Box::pin(async move {
                let resp = client
                    .post(&url)
                    .form(&[
                        ("username", username.as_str()),
                        ("password", password.as_str()),
                    ])
                    .send()
                    .await
                    .map_err(AuthenticatedClientError::from)?;

                AuthenticatedClient::ensure_authenticated(resp).map(|_| client)
            })
        })
    }
}

#[async_trait]
impl TorrentClient for QbittorrentClient {
    async fn download(&self, magnet: &Magnet) -> Result<(), TorrentClientError> {
        let url = format!("{}/api/v2/torrents/add", self.host);

        let resp = self
            .http
            .with_auth(async |client| {
                client
                    .post(&url)
                    .form(&[("urls", magnet.as_str())])
                    .send()
                    .await
            })
            .await
            .map_err(TorrentClientError::from)?;

        match resp.status() {
            reqwest::StatusCode::OK => Ok(()),
            s => Err(TorrentClientError::UnexpectedStatus(s)),
        }
    }

    async fn status(&self, info_hash: &str) -> Result<TorrentStatus, TorrentClientError> {
        let url = format!("{}/api/v2/torrents/info?hashes={}", self.host, info_hash);

        let resp = self
            .http
            .with_auth(async |client| client.get(&url).send().await)
            .await?;

        if !resp.status().is_success() {
            return Err(TorrentClientError::UnexpectedStatus(resp.status()));
        }

        let torrents: Vec<serde_json::Value> =
            resp.json().await.map_err(TorrentClientError::ClientError)?;

        let t = torrents
            .into_iter()
            .next()
            .ok_or_else(|| TorrentClientError::NotFound(info_hash.to_owned()))?;

        Ok(TorrentStatus {
            hash: t["hash"].as_str().unwrap_or("").to_owned(),
            state: parse_state(t["state"].as_str().unwrap_or("")),
            name: t["name"].as_str().unwrap_or("").to_owned(),
            content_name: t["content_path"]
                .as_str()
                .map(PathBuf::from)
                .and_then(|p| p.file_name().and_then(|f| f.to_str()).map(&str::to_owned))
                .unwrap_or("".to_string()),
        })
    }
}

fn parse_state(s: &str) -> TorrentState {
    match s {
        "downloading" | "stalledDL" | "checkingDL" | "forcedDL" | "metaDL" => {
            TorrentState::Downloading
        }
        "uploading" | "stalledUP" | "checkingUP" | "forcedUP" | "seeding" => TorrentState::Seeding,
        "pausedDL" | "pausedUP" | "stoppedDL" | "stoppedUP" => TorrentState::Paused,
        "error" | "missingFiles" | "unknown" => TorrentState::Error,
        _ => TorrentState::Unknown,
    }
}

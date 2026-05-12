use async_trait::async_trait;
use reqwest::{Client, StatusCode};
use tokio::sync::Mutex;

use crate::app::torrent::{TorrentClient, TorrentClientError};
use crate::types::{Magnet, TorrentState, TorrentStatus};

#[derive(Debug, Clone)]
pub struct QbittorrentConfig {
    pub host: String,
    pub username: String,
    pub password: String,
}

pub struct QbittorrentClient {
    cfg: QbittorrentConfig,
    http: Client,
    /// Cached session cookie value; refreshed on auth failure.
    session: Mutex<Option<String>>,
}

impl QbittorrentClient {
    pub fn new(cfg: QbittorrentConfig) -> Self {
        Self {
            cfg,
            http: Client::new(),
            session: Mutex::new(None),
        }
    }

    async fn login(&self) -> Result<String, TorrentClientError> {
        let url = format!("{}/api/v2/auth/login", self.cfg.host);
        let resp = self
            .http
            .post(&url)
            .form(&[
                ("username", self.cfg.username.as_str()),
                ("password", self.cfg.password.as_str()),
            ])
            .send()
            .await
            .map_err(|e| TorrentClientError::Api(e.to_string()))?;

        if resp.status() == StatusCode::FORBIDDEN {
            return Err(TorrentClientError::AuthFailed);
        }

        // Extract SID from Set-Cookie before consuming the response body.
        let sid = resp
            .headers()
            .get_all(reqwest::header::SET_COOKIE)
            .iter()
            .find_map(|v| {
                v.to_str().ok().and_then(|s| {
                    s.split(';')
                        .next()
                        .and_then(|p| p.trim().strip_prefix("SID=").map(str::to_owned))
                })
            })
            .ok_or(TorrentClientError::AuthFailed)?;

        let body = resp
            .text()
            .await
            .map_err(|e| TorrentClientError::Api(e.to_string()))?;

        if body.trim() != "Ok." {
            return Err(TorrentClientError::AuthFailed);
        }

        Ok(sid)
    }

    /// Returns a valid session cookie, logging in if needed.
    async fn session(&self) -> Result<String, TorrentClientError> {
        let mut guard = self.session.lock().await;
        if let Some(ref sid) = *guard {
            return Ok(sid.clone());
        }
        let sid = self.login().await?;
        *guard = Some(sid.clone());
        Ok(sid)
    }

    /// Invalidates the cached session, forcing a re-login next call.
    async fn invalidate_session(&self) {
        *self.session.lock().await = None;
    }

    /// Executes `f`, and if the response is 403 re-authenticates once and retries.
    async fn with_auth<F, Fut>(&self, f: F) -> Result<reqwest::Response, TorrentClientError>
    where
        F: Fn(String) -> Fut,
        Fut: std::future::Future<Output = Result<reqwest::Response, reqwest::Error>>,
    {
        let sid = self.session().await?;
        let resp = f(sid)
            .await
            .map_err(|e| TorrentClientError::Api(e.to_string()))?;

        if resp.status() == StatusCode::FORBIDDEN {
            self.invalidate_session().await;
            let sid = self.session().await?;
            return f(sid)
                .await
                .map_err(|e| TorrentClientError::Api(e.to_string()));
        }

        Ok(resp)
    }
}

#[async_trait]
impl TorrentClient for QbittorrentClient {
    async fn download(&self, magnet: &Magnet) -> Result<(), TorrentClientError> {
        let url = format!("{}/api/v2/torrents/add", self.cfg.host);
        let magnet_str = magnet.as_str().to_owned();
        let http = self.http.clone();

        let resp = self
            .with_auth(|sid| {
                let url = url.clone();
                let magnet_str = magnet_str.clone();
                let http = http.clone();
                async move {
                    http.post(&url)
                        .header("Cookie", format!("SID={sid}"))
                        .form(&[("urls", magnet_str.as_str())])
                        .send()
                        .await
                }
            })
            .await?;

        if !resp.status().is_success() {
            return Err(TorrentClientError::Api(format!(
                "add torrent failed: HTTP {}",
                resp.status()
            )));
        }

        Ok(())
    }

    async fn status(&self, info_hash: &str) -> Result<TorrentStatus, TorrentClientError> {
        let url = format!(
            "{}/api/v2/torrents/info?hashes={}",
            self.cfg.host, info_hash
        );
        let http = self.http.clone();

        let resp = self
            .with_auth(|sid| {
                let url = url.clone();
                let http = http.clone();
                async move {
                    http.get(&url)
                        .header("Cookie", format!("SID={sid}"))
                        .send()
                        .await
                }
            })
            .await?;

        let torrents: Vec<serde_json::Value> = resp
            .json()
            .await
            .map_err(|e| TorrentClientError::Api(e.to_string()))?;

        let t = torrents
            .into_iter()
            .next()
            .ok_or_else(|| TorrentClientError::NotFound(info_hash.to_owned()))?;

        Ok(TorrentStatus {
            hash: t["hash"].as_str().unwrap_or("").to_owned(),
            state: parse_state(t["state"].as_str().unwrap_or("")),
            name: t["name"].as_str().unwrap_or("").to_owned(),
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

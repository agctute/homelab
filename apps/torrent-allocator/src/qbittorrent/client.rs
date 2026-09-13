use anyhow::{Context, Result};
use reqwest::Client;
use serde::Deserialize;
use tracing::debug;

pub struct QbittorrentClient {
    http: Client,
    base_url: String,
}

/// Subset of qBittorrent's `/api/v2/torrents/info` response we care about.
#[derive(Debug, Clone, Deserialize)]
pub struct TorrentInfo {
    pub hash: String,
    pub name: String,
    #[serde(default)]
    pub category: String,
    pub save_path: String,
    /// Full path to the downloaded content on disk (file or folder).
    #[serde(default)]
    pub content_path: String,
    pub state: String,
}

impl QbittorrentClient {
    /// Create and authenticate a client. Returns Err if login fails.
    pub async fn connect(base_url: &str, username: &str, password: &str) -> Result<Self> {
        let http = Client::builder().cookie_store(true).build()?;

        let login_url = format!("{base_url}/api/v2/auth/login");
        let resp = http
            .post(&login_url)
            .form(&[("username", username), ("password", password)])
            .send()
            .await
            .context("qBittorrent login request")?;

        let body = resp.text().await.unwrap_or_default();
        if body.trim() != "Ok." {
            anyhow::bail!("qBittorrent login failed: {body}");
        }

        debug!("qBittorrent logged in at {base_url}");
        Ok(Self {
            http,
            base_url: base_url.to_string(),
        })
    }

    /// Torrents that have finished downloading (now seeding, paused-after-completion,
    /// or otherwise done). qBittorrent is the source of truth for "completed" — a raw
    /// directory listing can't tell a finished torrent apart from one that's still
    /// downloading or stalled.
    pub async fn list_completed(&self) -> Result<Vec<TorrentInfo>> {
        let url = format!("{}/api/v2/torrents/info", self.base_url);
        let resp = self
            .http
            .get(&url)
            .query(&[("filter", "completed")])
            .send()
            .await
            .context("qBittorrent list torrents")?;

        let torrents: Vec<TorrentInfo> = resp
            .json()
            .await
            .context("parse qBittorrent torrent list")?;

        debug!(count = torrents.len(), "fetched completed torrents");
        Ok(torrents)
    }
}

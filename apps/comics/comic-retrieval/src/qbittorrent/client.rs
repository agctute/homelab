use anyhow::{Context, Result};
use reqwest::Client;
use tracing::debug;

pub struct QbittorrentClient {
    http: Client,
    base_url: String,
}

impl QbittorrentClient {
    /// Create and authenticate a client. Returns Err if login fails.
    pub async fn connect(base_url: &str, username: &str, password: &str) -> Result<Self> {
        let http = Client::builder()
            .cookie_store(true)
            .timeout(std::time::Duration::from_secs(20))
            .build()?;

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
        Ok(Self { http, base_url: base_url.to_string() })
    }

    /// Add a torrent by URL (direct .torrent link or magnet URI).
    /// `save_path` is the folder on the NAS where the torrent should download.
    pub async fn add_torrent(&self, torrent_url: &str, save_path: &str) -> Result<()> {
        self.add_torrent_with_category(torrent_url, save_path, "manga").await
    }

    /// Same as `add_torrent`, but tags the torrent with a specific category —
    /// used to distinguish single-chapter fallback downloads (category
    /// "manga") from historical-backlog backfills (category
    /// "manga-backfill"), so the backfill reconciler can find just its own
    /// torrents via `list_torrents`.
    pub async fn add_torrent_with_category(
        &self,
        torrent_url: &str,
        save_path: &str,
        category: &str,
    ) -> Result<()> {
        let url = format!("{}/api/v2/torrents/add", self.base_url);
        let resp = self
            .http
            .post(&url)
            .form(&[
                ("urls", torrent_url),
                ("savepath", save_path),
                ("category", category),
            ])
            .send()
            .await
            .context("qBittorrent add torrent")?;

        let body = resp.text().await.unwrap_or_default();
        if body.trim() != "Ok." {
            anyhow::bail!("qBittorrent add failed: {body}");
        }

        debug!("torrent added: {torrent_url} → {save_path} [{category}]");
        Ok(())
    }

    /// Lists torrents in a given category, most recently useful for finding
    /// backfill torrents added by `add_torrent_with_category`.
    pub async fn list_torrents(&self, category: &str) -> Result<Vec<TorrentInfo>> {
        let url = format!(
            "{}/api/v2/torrents/info?category={}",
            self.base_url,
            urlencoding::encode(category)
        );
        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .context("qBittorrent list torrents")?;
        let body: Vec<serde_json::Value> = resp.json().await.context("parse torrent list")?;

        Ok(body
            .into_iter()
            .filter_map(|v| {
                Some(TorrentInfo {
                    hash: v["hash"].as_str()?.to_string(),
                    name: v.get("name").and_then(|n| n.as_str()).unwrap_or_default().to_string(),
                    progress: v.get("progress").and_then(|p| p.as_f64()).unwrap_or(0.0),
                    save_path: v
                        .get("save_path")
                        .and_then(|p| p.as_str())
                        .unwrap_or_default()
                        .to_string(),
                })
            })
            .collect())
    }
}

#[derive(Debug, Clone)]
pub struct TorrentInfo {
    pub hash: String,
    pub name: String,
    /// 0.0 to 1.0; 1.0 means the download is complete (seeding/uploading).
    pub progress: f64,
    pub save_path: String,
}

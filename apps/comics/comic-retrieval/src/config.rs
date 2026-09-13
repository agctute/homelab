use anyhow::{Context, Result};

#[derive(Debug, Clone)]
pub struct Config {
    pub mangadex_username: String,
    pub mangadex_password: String,
    pub mangadex_client_id: String,
    pub mangadex_client_secret: String,
    pub nas_comics_path: String,
    pub db_path: String,
    pub concurrent_downloads: usize,
    pub max_api_rps: u64,
    pub run_catalog_refresh_on_start: bool,
    pub run_daily_update_on_start: bool,
    // nyaa / qbittorrent fallback
    pub nyaa_enabled: bool,
    pub qbittorrent_url: String,
    pub qbittorrent_username: String,
    pub qbittorrent_password: String,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        Ok(Self {
            mangadex_username: env("MANGADEX_USERNAME")?,
            mangadex_password: env("MANGADEX_PASSWORD")?,
            mangadex_client_id: env("MANGADEX_CLIENT_ID")?,
            mangadex_client_secret: env("MANGADEX_CLIENT_SECRET")?,
            nas_comics_path: env("NAS_COMICS_PATH")
                .unwrap_or_else(|_| "/books/manga".to_string()),
            db_path: env("DB_PATH").unwrap_or_else(|_| "/comics/state.db".to_string()),
            concurrent_downloads: env("CONCURRENT_DOWNLOADS")
                .unwrap_or_else(|_| "3".to_string())
                .parse()
                .context("CONCURRENT_DOWNLOADS must be a number")?,
            max_api_rps: env("MAX_API_RPS")
                .unwrap_or_else(|_| "5".to_string())
                .parse()
                .context("MAX_API_RPS must be a number")?,
            run_catalog_refresh_on_start: env("RUN_CATALOG_REFRESH_ON_START")
                .unwrap_or_else(|_| "false".to_string())
                == "true",
            run_daily_update_on_start: env("RUN_DAILY_UPDATE_ON_START")
                .unwrap_or_else(|_| "false".to_string())
                == "true",
            nyaa_enabled: env("NYAA_ENABLED")
                .unwrap_or_else(|_| "true".to_string())
                == "true",
            qbittorrent_url: env("QBITTORRENT_URL")
                .unwrap_or_else(|_| "http://10.0.0.41:8090".to_string()),
            qbittorrent_username: env("QBITTORRENT_USERNAME")
                .unwrap_or_else(|_| "admin".to_string()),
            qbittorrent_password: env("QBITTORRENT_PASSWORD")
                .unwrap_or_else(|_| "adminadmin".to_string()),
        })
    }
}

fn env(key: &str) -> Result<String> {
    std::env::var(key).with_context(|| format!("missing env var {key}"))
}

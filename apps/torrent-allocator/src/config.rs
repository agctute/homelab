use anyhow::{Context, Result};

#[derive(Debug, Clone)]
pub struct Config {
    pub qbittorrent_url: String,
    pub qbittorrent_username: String,
    pub qbittorrent_password: String,
    pub db_path: String,
    pub poll_interval_secs: u64,
    pub openai_api_key: String,
    pub openai_model: String,

    // Library destinations. Defaults assume the NAS books/torrents shares
    // are mounted inside this pod at these paths — see k8s/pvc.yaml and
    // k8s/deployment.yaml.
    pub comics_path: String,
    pub manga_path: String,
    pub novels_path: String,
    pub textbooks_path: String,
    pub audiobooks_path: String,

    // External conversion tools, invoked as subprocesses when a torrent's
    // files aren't already in the target format. See src/format/convert.rs.
    pub zip_bin: String,
    pub unrar_bin: String,
    pub ebook_convert_bin: String,

    // Optional: raises Google Books' free-tier rate limit. Works fine unset.
    pub google_books_api_key: Option<String>,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        Ok(Self {
            qbittorrent_url: env_or("QBITTORRENT_URL", "http://qbittorrent:8090"),
            qbittorrent_username: env_or("QBITTORRENT_USERNAME", "admin"),
            qbittorrent_password: env("QBITTORRENT_PASSWORD")?,
            db_path: env_or("DB_PATH", "/data/state.db"),
            poll_interval_secs: env_or("POLL_INTERVAL_SECS", "30")
                .parse()
                .context("POLL_INTERVAL_SECS must be a number")?,
            openai_api_key: env("OPENAI_API_KEY")?,
            openai_model: env_or("OPENAI_MODEL", "gpt-4o-mini"),
            comics_path: env_or("COMICS_PATH", "/books/comics"),
            manga_path: env_or("MANGA_PATH", "/books/manga"),
            novels_path: env_or("NOVELS_PATH", "/books/novels"),
            textbooks_path: env_or("TEXTBOOKS_PATH", "/books/textbooks"),
            audiobooks_path: env_or("AUDIOBOOKS_PATH", "/books/audiobooks"),
            zip_bin: env_or("ZIP_BIN", "zip"),
            unrar_bin: env_or("UNRAR_BIN", "unrar"),
            ebook_convert_bin: env_or("EBOOK_CONVERT_BIN", "ebook-convert"),
            google_books_api_key: env_opt("GOOGLE_BOOKS_API_KEY"),
        })
    }
}

fn env(key: &str) -> Result<String> {
    std::env::var(key).with_context(|| format!("missing env var {key}"))
}

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

fn env_opt(key: &str) -> Option<String> {
    std::env::var(key).ok()
}

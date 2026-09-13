use super::BookMetadata;
use anyhow::{Context, Result};
use reqwest::Client;
use serde::Deserialize;
use tracing::debug;

const BASE_URL: &str = "https://www.googleapis.com/books/v1/volumes";
const DESCRIPTION_MAX_CHARS: usize = 300;

pub struct GoogleBooksClient {
    http: Client,
    api_key: Option<String>,
}

impl GoogleBooksClient {
    pub fn new(api_key: Option<String>) -> Self {
        Self {
            http: Client::new(),
            api_key,
        }
    }

    /// Best-effort lookup: returns `None` on any failure (no match, network
    /// error, rate limit, ...) rather than propagating an error, since this
    /// is purely an enrichment step — classification must proceed fine
    /// without it.
    pub async fn search(&self, query: &str) -> Option<BookMetadata> {
        match self.try_search(query).await {
            Ok(result) => result,
            Err(e) => {
                debug!(query, "google books lookup failed: {e:#}");
                None
            }
        }
    }

    async fn try_search(&self, query: &str) -> Result<Option<BookMetadata>> {
        if query.trim().is_empty() {
            return Ok(None);
        }

        let mut req = self
            .http
            .get(BASE_URL)
            .query(&[("q", query), ("maxResults", "1")]);
        if let Some(key) = &self.api_key {
            req = req.query(&[("key", key.as_str())]);
        }

        let resp = req.send().await.context("google books request failed")?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            anyhow::bail!("google books API error ({status}): {text}");
        }

        let parsed: SearchResponse =
            serde_json::from_str(&text).context("parse google books response")?;

        Ok(parsed.items.into_iter().next().map(|item| BookMetadata {
            title: item.volume_info.title,
            authors: item.volume_info.authors.unwrap_or_default(),
            publisher: item.volume_info.publisher,
            description: item.volume_info.description.map(|d| truncate(&d)),
            categories: item.volume_info.categories.unwrap_or_default(),
            language: item.volume_info.language,
        }))
    }
}

fn truncate(s: &str) -> String {
    if s.len() <= DESCRIPTION_MAX_CHARS {
        return s.to_string();
    }
    match s.char_indices().nth(DESCRIPTION_MAX_CHARS) {
        Some((idx, _)) => format!("{}…", &s[..idx]),
        None => s.to_string(),
    }
}

#[derive(Deserialize, Default)]
struct SearchResponse {
    #[serde(default)]
    items: Vec<Item>,
}

#[derive(Deserialize)]
struct Item {
    #[serde(rename = "volumeInfo")]
    volume_info: VolumeInfo,
}

#[derive(Deserialize, Default)]
struct VolumeInfo {
    title: Option<String>,
    authors: Option<Vec<String>>,
    publisher: Option<String>,
    description: Option<String>,
    categories: Option<Vec<String>>,
    language: Option<String>,
}

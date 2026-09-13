use anyhow::{Context, Result};
use std::path::PathBuf;
use tempfile::TempDir;
use tracing::{debug, info, warn};

use super::client::MangaDexClient;
use super::models::{AtHomeResponse, Chapter};

pub struct DownloadedChapter {
    pub chapter_id: String,
    pub manga_id: String,
    pub chapter_num: Option<String>,
    pub volume_num: Option<String>,
    pub manga_title: String,
    pub pages_dir: TempDir,
    pub page_files: Vec<PathBuf>,
}

pub async fn download_chapter(
    client: &MangaDexClient,
    chapter_id: &str,
    manga_title: &str,
    manga_id: &str,
) -> Result<DownloadedChapter> {
    // get chapter metadata
    let chapter_url = client.api_url(&format!("/chapter/{chapter_id}"));
    let resp: serde_json::Value = client
        .get_json(&chapter_url)
        .await
        .context("fetch chapter metadata")?;
    let chapter: Chapter = serde_json::from_value(resp["data"].clone())
        .context("parse chapter metadata")?;

    if chapter.attributes.pages == 0 {
        anyhow::bail!("chapter {chapter_id} has 0 pages");
    }
    if chapter.attributes.external_url.is_some() {
        anyhow::bail!("chapter {chapter_id} is external-only");
    }

    // get MD@Home server
    let at_home_url = client.api_url(&format!("/at-home/server/{chapter_id}"));
    let at_home: AtHomeResponse = client
        .get_json(&at_home_url)
        .await
        .context("fetch at-home server")?;

    info!(
        chapter_id,
        pages = at_home.chapter.data.len(),
        "downloading chapter"
    );

    let pages_dir = TempDir::new().context("create temp dir")?;
    let mut page_files = Vec::with_capacity(at_home.chapter.data.len());

    for (idx, filename) in at_home.chapter.data.iter().enumerate() {
        let page_url = format!(
            "{}/data/{}/{}",
            at_home.base_url, at_home.chapter.hash, filename
        );

        let ext = PathBuf::from(filename)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("jpg")
            .to_string();

        let page_path = pages_dir.path().join(format!("{:04}.{ext}", idx + 1));

        let data = download_page_with_retry(client, &page_url).await
            .with_context(|| format!("page {idx} of chapter {chapter_id}"))?;

        tokio::fs::write(&page_path, &data).await?;
        page_files.push(page_path);
        debug!("page {}/{}", idx + 1, at_home.chapter.data.len());
    }

    Ok(DownloadedChapter {
        chapter_id: chapter.id,
        manga_id: manga_id.to_string(),
        chapter_num: chapter.attributes.chapter.clone(),
        volume_num: chapter.attributes.volume.clone(),
        manga_title: manga_title.to_string(),
        pages_dir,
        page_files,
    })
}

async fn download_page_with_retry(client: &MangaDexClient, url: &str) -> Result<bytes::Bytes> {
    for attempt in 0..3u32 {
        match client.get_bytes(url).await {
            Ok(b) => return Ok(b),
            Err(e) if attempt < 2 => {
                warn!("page download attempt {attempt} failed: {e}, retrying");
                tokio::time::sleep(std::time::Duration::from_secs(2u64.pow(attempt))).await;
            }
            Err(e) => return Err(e),
        }
    }
    unreachable!()
}

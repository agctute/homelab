use std::sync::Arc;
use std::time::Duration;
use tracing::{error, info, warn};

use crate::config::Config;
use crate::db::queries;
use crate::library::{cbz, router};
use crate::mangadex::{client::MangaDexClient, download};
use crate::nyaa::search as nyaa;
use crate::qbittorrent::client::QbittorrentClient;

const MAX_ATTEMPTS: i64 = 5;

pub async fn run(client: Arc<MangaDexClient>, pool: sqlx::SqlitePool, config: Arc<Config>) {
    info!("download worker started");
    let http = reqwest::Client::builder()
        .user_agent("comic-retrieval/0.1")
        .timeout(Duration::from_secs(15))
        .build()
        .expect("build http client");

    loop {
        match process_one(&client, &pool, &config, &http).await {
            Ok(true) => {}
            Ok(false) => {
                tokio::time::sleep(Duration::from_secs(30)).await;
            }
            Err(e) => {
                error!("worker error: {e:#}");
                tokio::time::sleep(Duration::from_secs(10)).await;
            }
        }
    }
}

async fn process_one(
    client: &MangaDexClient,
    pool: &sqlx::SqlitePool,
    config: &Config,
    http: &reqwest::Client,
) -> anyhow::Result<bool> {
    let item = match queries::next_queue_item(pool).await? {
        Some(i) => i,
        None => return Ok(false),
    };

    let catalog_row = sqlx::query_as::<_, (String,)>(
        "SELECT title FROM catalog WHERE manga_id = ?1",
    )
    .bind(&item.manga_id)
    .fetch_optional(pool)
    .await?;

    let (title,) = match catalog_row {
        Some(r) => r,
        None => {
            warn!("chapter {} has no catalog entry, skipping", item.chapter_id);
            queries::dequeue(pool, item.id).await?;
            return Ok(true);
        }
    };

    let safe_title = router::sanitize_title(&title);
    info!(chapter_id = %item.chapter_id, manga = %safe_title, "processing download");

    let downloaded = match download::download_chapter(
        client,
        &item.chapter_id,
        &safe_title,
        &item.manga_id,
    )
    .await
    {
        Ok(d) => d,
        Err(e) => {
            warn!("download failed for {}: {e:#}", item.chapter_id);
            let new_attempts = item.attempts + 1;
            queries::queue_item_failed(pool, item.id, &format!("{e:#}")).await?;

            if new_attempts >= MAX_ATTEMPTS && config.nyaa_enabled {
                try_nyaa_fallback(pool, config, http, &item.manga_id, &safe_title).await;
            }
            return Ok(true);
        }
    };

    // A different scanlation group's release of this exact (volume, chapter)
    // may already be downloaded — the CBZ filename is derived only from
    // title/volume/chapter, so writing this one too would silently overwrite
    // it. This is normally caught before queuing (see mangadex::catalog); this
    // is a backstop for anything queued before that check existed, or races.
    if let Some(existing_path) = queries::cbz_path_for_slot(
        pool,
        &item.manga_id,
        downloaded.volume_num.as_deref(),
        downloaded.chapter_num.as_deref(),
    )
    .await?
    {
        warn!(
            chapter_id = %item.chapter_id,
            manga = %safe_title,
            chapter = downloaded.chapter_num.as_deref().unwrap_or("?"),
            existing = %existing_path,
            "duplicate translation of an already-downloaded chapter, skipping write"
        );
        queries::mark_chapter_downloaded(
            pool,
            &item.chapter_id,
            &item.manga_id,
            downloaded.chapter_num.as_deref(),
            downloaded.volume_num.as_deref(),
            &existing_path,
        )
        .await?;
        queries::dequeue(pool, item.id).await?;
        return Ok(true);
    }

    let filename = cbz::cbz_filename(
        &safe_title,
        downloaded.volume_num.as_deref(),
        downloaded.chapter_num.as_deref(),
    );

    let manga_folder = std::path::Path::new(&config.nas_comics_path).join(&safe_title);
    let cbz_path = manga_folder.join(&filename);

    let meta = cbz::CbzMeta {
        title: &filename,
        volume: downloaded.volume_num.as_deref(),
        chapter: downloaded.chapter_num.as_deref(),
        series: &safe_title,
    };

    if let Err(e) = cbz::create(&downloaded.page_files, &cbz_path, &meta) {
        warn!("cbz creation failed for {}: {e:#}", item.chapter_id);
        queries::queue_item_failed(pool, item.id, &format!("{e:#}")).await?;
        return Ok(true);
    }

    queries::mark_chapter_downloaded(
        pool,
        &item.chapter_id,
        &item.manga_id,
        downloaded.chapter_num.as_deref(),
        downloaded.volume_num.as_deref(),
        cbz_path.to_str().unwrap_or(""),
    )
    .await?;
    queries::dequeue(pool, item.id).await?;

    let remaining = queries::queue_size(pool).await?;
    info!(
        chapter = downloaded.chapter_num.as_deref().unwrap_or("?"),
        cbz = %cbz_path.display(),
        queue_remaining = remaining,
        "chapter saved"
    );

    Ok(true)
}

async fn try_nyaa_fallback(
    pool: &sqlx::SqlitePool,
    config: &Config,
    http: &reqwest::Client,
    manga_id: &str,
    title: &str,
) {
    // only attempt once per manga
    match queries::nyaa_already_attempted(pool, manga_id).await {
        Ok(true) => return,
        Err(e) => { warn!("nyaa check failed: {e}"); return; }
        Ok(false) => {}
    }

    info!(manga = title, "MangaDex exhausted — searching nyaa.si");

    let results = match nyaa::search(http, title).await {
        Ok(r) => r,
        Err(e) => {
            warn!("nyaa search failed for '{title}': {e:#}");
            return;
        }
    };

    let best = match nyaa::best_match(&results, title) {
        Some(r) => r,
        None => {
            warn!("no nyaa match found for '{title}'");
            let _ = queries::mark_nyaa_attempted(pool, manga_id).await;
            return;
        }
    };

    info!(
        torrent = %best.title,
        seeders = best.seeders,
        "found nyaa torrent for '{title}'"
    );

    let save_path = std::path::Path::new(&config.nas_comics_path)
        .join(title)
        .to_string_lossy()
        .into_owned();

    match QbittorrentClient::connect(
        &config.qbittorrent_url,
        &config.qbittorrent_username,
        &config.qbittorrent_password,
    )
    .await
    {
        Ok(qbt) => {
            match qbt.add_torrent(&best.torrent_url, &save_path).await {
                Ok(()) => info!(
                    torrent = %best.title,
                    save_path = %save_path,
                    "torrent added to qBittorrent"
                ),
                Err(e) => warn!("qBittorrent add failed: {e:#}"),
            }
        }
        Err(e) => warn!("qBittorrent connect failed: {e:#}"),
    }

    let _ = queries::mark_nyaa_attempted(pool, manga_id).await;
}

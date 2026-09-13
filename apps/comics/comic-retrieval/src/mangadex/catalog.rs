use anyhow::Result;
use serde::Deserialize;
use std::collections::HashMap;
use tracing::{info, warn};

use super::client::MangaDexClient;
use super::models::{Chapter, Manga};
use crate::db::queries;
use sqlx::SqlitePool;

const PAGE_SIZE: u64 = 100;
const EXCLUDED_ORIGINS: &[&str] = &["zh", "zh-hk"];
// MangaDex tag UUIDs (format group) for content this catalog shouldn't
// auto-pick-up: unofficial fan works, not the official releases the
// downloader is meant to track. Looked up via GET /manga/tag.
const TAG_DOUJINSHI: &str = "b13b2a48-c720-44a9-9c77-39c9979373fb";
const TAG_FAN_COLORED: &str = "7b2ce280-79ef-4c09-9b58-12b7c23a9b78";

#[derive(Deserialize)]
struct MangaListResponse {
    data: Vec<Manga>,
    total: u64,
}

pub async fn refresh_catalog(client: &MangaDexClient, pool: &SqlitePool) -> Result<usize> {
    info!("starting yearly catalog refresh");

    let follows_map = fetch_top_percent(client, "followedCount").await?;
    info!("top-follows list: {} manga", follows_map.len());

    let rating_map = fetch_top_percent(client, "rating").await?;
    info!("top-rating list: {} manga", rating_map.len());

    // union: merge maps, tracking which list each manga appears in
    let mut union: HashMap<String, (Manga, bool, bool)> = HashMap::new();
    for (id, manga) in follows_map {
        union.insert(id, (manga, true, false));
    }
    for (id, manga) in rating_map {
        union
            .entry(id)
            .and_modify(|(_, _, in_rating)| *in_rating = true)
            .or_insert((manga, false, true));
    }

    info!("union: {} unique manga", union.len());

    let mut added = 0usize;
    for (_, (manga, in_follows, in_rating)) in union {
        if EXCLUDED_ORIGINS.contains(&manga.attributes.original_language.as_str()) {
            continue;
        }
        let title = manga.dual_title().combined;
        queries::upsert_catalog(
            pool,
            &manga.id,
            &title,
            &manga.attributes.original_language,
            &manga.attributes.status,
            in_follows,
            in_rating,
        )
        .await?;
        added += 1;
    }

    info!("catalog refresh complete: {added} manga upserted");
    Ok(added)
}

async fn fetch_top_percent(
    client: &MangaDexClient,
    order_field: &str,
) -> Result<HashMap<String, Manga>> {
    let url = build_list_url(client, order_field, PAGE_SIZE, 0);
    let first: MangaListResponse = client.get_json(&url).await?;
    let total = first.total;
    let threshold = (total as f64 * 0.01).ceil() as u64;

    info!("order[{order_field}]: total={total}, fetching top {threshold}");

    let mut map = HashMap::new();
    collect_manga(first.data, &mut map);

    let mut offset = PAGE_SIZE;
    while offset < threshold {
        let url = build_list_url(client, order_field, PAGE_SIZE, offset);
        match fetch_page_with_retry(client, &url).await {
            Ok(page) => {
                if page.data.is_empty() {
                    break;
                }
                collect_manga(page.data, &mut map);
            }
            Err(e) => {
                warn!("page offset={offset} failed after retries: {e}, stopping");
                break;
            }
        }
        offset += PAGE_SIZE;
    }

    Ok(map)
}

async fn fetch_page_with_retry(client: &MangaDexClient, url: &str) -> Result<MangaListResponse> {
    for attempt in 0..3u32 {
        match client.get_json::<MangaListResponse>(url).await {
            Ok(page) => return Ok(page),
            Err(e) if attempt < 2 => {
                warn!("page fetch attempt {attempt} failed: {e}, retrying");
                tokio::time::sleep(std::time::Duration::from_secs(2u64.pow(attempt))).await;
            }
            Err(e) => return Err(e),
        }
    }
    unreachable!()
}

fn build_list_url(client: &MangaDexClient, order_field: &str, limit: u64, offset: u64) -> String {
    client.api_url(&format!(
        "/manga?order%5B{order_field}%5D=desc\
         &originalLanguage%5B%5D=ja\
         &originalLanguage%5B%5D=ko\
         &limit={limit}\
         &offset={offset}\
         &availableTranslatedLanguage%5B%5D=en\
         &contentRating%5B%5D=safe\
         &contentRating%5B%5D=suggestive\
         &contentRating%5B%5D=erotica\
         &excludedTags%5B%5D={TAG_DOUJINSHI}\
         &excludedTags%5B%5D={TAG_FAN_COLORED}"
    ))
}

fn collect_manga(manga: Vec<Manga>, map: &mut HashMap<String, Manga>) {
    for m in manga {
        if !EXCLUDED_ORIGINS.contains(&m.attributes.original_language.as_str()) {
            map.entry(m.id.clone()).or_insert(m);
        }
    }
}

/// Fetch all English chapter IDs for a manga and insert into download_queue.
pub async fn queue_new_manga_chapters(
    client: &MangaDexClient,
    pool: &SqlitePool,
    manga_id: &str,
    priority: i64,
) -> Result<usize> {
    let mut queued = 0usize;
    let mut offset = 0u64;

    loop {
        let url = client.api_url(&format!(
            "/manga/{manga_id}/feed\
             ?translatedLanguage%5B%5D=en\
             &limit=500\
             &offset={offset}\
             &order%5Bchapter%5D=asc\
             &includeFuturePublishAt=0\
             &includeEmptyPages=0"
        ));
        let resp: serde_json::Value = client.get_json(&url).await?;
        let total = resp["total"].as_u64().unwrap_or(0);
        let chapters: Vec<Chapter> =
            serde_json::from_value(resp["data"].clone()).unwrap_or_default();

        if chapters.is_empty() {
            break;
        }

        let n = chapters.len();
        for ch in chapters {
            if maybe_enqueue(pool, manga_id, priority, &ch).await? {
                queued += 1;
            }
        }

        offset += n as u64;
        if offset >= total {
            break;
        }
    }

    Ok(queued)
}

/// Enqueues a chapter unless it's already downloaded/queued, or a different
/// scanlation group's release of the same (volume, chapter) slot already is —
/// MangaDex commonly lists several groups' translations of the same chapter,
/// and downloading more than one would silently clobber the same CBZ file on
/// disk (the filename is derived only from title/volume/chapter). First
/// release seen wins; later duplicates are skipped.
async fn maybe_enqueue(
    pool: &SqlitePool,
    manga_id: &str,
    priority: i64,
    ch: &Chapter,
) -> Result<bool> {
    if ch.attributes.external_url.is_some() {
        return Ok(false);
    }
    if queries::chapter_downloaded(pool, &ch.id).await? {
        return Ok(false);
    }
    let (chapter_num, volume_num) = (
        ch.attributes.chapter.as_deref(),
        ch.attributes.volume.as_deref(),
    );
    if queries::chapter_slot_downloaded(pool, manga_id, volume_num, chapter_num).await?
        || queries::chapter_slot_queued(pool, manga_id, volume_num, chapter_num).await?
    {
        info!(
            chapter_id = %ch.id,
            manga_id,
            chapter = chapter_num.unwrap_or("?"),
            volume = volume_num.unwrap_or("?"),
            "skipping duplicate translation of an already-queued/downloaded chapter"
        );
        return Ok(false);
    }
    queries::enqueue_chapter(pool, &ch.id, manga_id, priority, chapter_num, volume_num).await?;
    Ok(true)
}

/// Queue chapters updated since a given timestamp for all catalog manga.
pub async fn queue_daily_updates(
    client: &MangaDexClient,
    pool: &SqlitePool,
    since: &str,
) -> Result<usize> {
    let catalog = queries::all_catalog(pool).await?;
    let mut queued = 0usize;

    for entry in &catalog {
        let url = client.api_url(&format!(
            "/manga/{}/feed\
             ?translatedLanguage%5B%5D=en\
             &updatedAtSince={}\
             &limit=100\
             &order%5Bchapter%5D=asc\
             &includeFuturePublishAt=0\
             &includeEmptyPages=0",
            entry.manga_id, since
        ));
        let resp: serde_json::Value = match client.get_json(&url).await {
            Ok(v) => v,
            Err(e) => {
                warn!("daily feed for {}: {e}", entry.manga_id);
                continue;
            }
        };
        let chapters: Vec<Chapter> =
            serde_json::from_value(resp["data"].clone()).unwrap_or_default();

        for ch in chapters {
            if maybe_enqueue(pool, &entry.manga_id, 10, &ch).await? {
                queued += 1;
            }
        }
    }

    Ok(queued)
}

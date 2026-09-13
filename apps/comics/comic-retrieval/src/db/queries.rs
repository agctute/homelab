use anyhow::Result;
use chrono::Utc;
use sqlx::SqlitePool;
use std::collections::HashSet;

#[derive(Debug)]
pub struct CatalogEntry {
    pub manga_id: String,
    pub title: String,
    pub origin: String,
    pub status: String,
    pub in_follows: i64,
    pub in_rating: i64,
}

#[derive(Debug)]
pub struct QueueItem {
    pub id: i64,
    pub chapter_id: String,
    pub manga_id: String,
    pub priority: i64,
    pub attempts: i64,
}

pub async fn upsert_catalog(
    pool: &SqlitePool,
    manga_id: &str,
    title: &str,
    origin: &str,
    status: &str,
    in_follows: bool,
    in_rating: bool,
) -> Result<()> {
    let now = Utc::now().to_rfc3339();
    let in_follows = in_follows as i64;
    let in_rating = in_rating as i64;
    sqlx::query(
        r#"
        INSERT INTO catalog (manga_id, title, origin, status, in_follows, in_rating, added_at)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
        ON CONFLICT(manga_id) DO UPDATE SET
            title      = excluded.title,
            status     = excluded.status,
            in_follows = MAX(in_follows, excluded.in_follows),
            in_rating  = MAX(in_rating,  excluded.in_rating)
        "#,
    )
    .bind(manga_id)
    .bind(title)
    .bind(origin)
    .bind(status)
    .bind(in_follows)
    .bind(in_rating)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn update_manga_status(pool: &SqlitePool, manga_id: &str, status: &str) -> Result<()> {
    sqlx::query("UPDATE catalog SET status = ?1 WHERE manga_id = ?2")
        .bind(status)
        .bind(manga_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn all_catalog(pool: &SqlitePool) -> Result<Vec<CatalogEntry>> {
    let rows = sqlx::query_as::<_, (String, String, String, String, i64, i64)>(
        "SELECT manga_id, title, origin, status, in_follows, in_rating FROM catalog",
    )
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|(manga_id, title, origin, status, in_follows, in_rating)| CatalogEntry {
        manga_id,
        title,
        origin,
        status,
        in_follows,
        in_rating,
    })
    .collect();
    Ok(rows)
}

pub async fn chapter_downloaded(pool: &SqlitePool, chapter_id: &str) -> Result<bool> {
    let row: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM downloaded_chapters WHERE chapter_id = ?1")
            .bind(chapter_id)
            .fetch_one(pool)
            .await?;
    Ok(row.0 > 0)
}

pub async fn manga_has_any_download(pool: &SqlitePool, manga_id: &str) -> Result<bool> {
    let row: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM downloaded_chapters WHERE manga_id = ?1")
            .bind(manga_id)
            .fetch_one(pool)
            .await?;
    Ok(row.0 > 0)
}

/// True if some *other* chapter_id already claims this manga's (volume, chapter)
/// slot — i.e. a different scanlation group's release of the same chapter has
/// already been downloaded. Slot-less chapters (no chapter number at all) are
/// never considered duplicates of each other.
pub async fn chapter_slot_downloaded(
    pool: &SqlitePool,
    manga_id: &str,
    volume_num: Option<&str>,
    chapter_num: Option<&str>,
) -> Result<bool> {
    let Some(chapter_num) = chapter_num else {
        return Ok(false);
    };
    let row: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM downloaded_chapters
         WHERE manga_id = ?1 AND chapter_num = ?2 AND volume_num IS ?3",
    )
    .bind(manga_id)
    .bind(chapter_num)
    .bind(volume_num)
    .fetch_one(pool)
    .await?;
    Ok(row.0 > 0)
}

/// True if this manga's (volume, chapter) slot is already sitting in the
/// download queue under some chapter_id.
pub async fn chapter_slot_queued(
    pool: &SqlitePool,
    manga_id: &str,
    volume_num: Option<&str>,
    chapter_num: Option<&str>,
) -> Result<bool> {
    let Some(chapter_num) = chapter_num else {
        return Ok(false);
    };
    let row: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM download_queue
         WHERE manga_id = ?1 AND chapter_num = ?2 AND volume_num IS ?3",
    )
    .bind(manga_id)
    .bind(chapter_num)
    .bind(volume_num)
    .fetch_one(pool)
    .await?;
    Ok(row.0 > 0)
}

/// cbz_path already on record for this manga's (volume, chapter) slot, if any —
/// used to point a duplicate release at the file that's already there instead
/// of overwriting it.
pub async fn cbz_path_for_slot(
    pool: &SqlitePool,
    manga_id: &str,
    volume_num: Option<&str>,
    chapter_num: Option<&str>,
) -> Result<Option<String>> {
    let Some(chapter_num) = chapter_num else {
        return Ok(None);
    };
    let row: Option<(String,)> = sqlx::query_as(
        "SELECT cbz_path FROM downloaded_chapters
         WHERE manga_id = ?1 AND chapter_num = ?2 AND volume_num IS ?3
         LIMIT 1",
    )
    .bind(manga_id)
    .bind(chapter_num)
    .bind(volume_num)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|(p,)| p))
}

pub async fn mark_chapter_downloaded(
    pool: &SqlitePool,
    chapter_id: &str,
    manga_id: &str,
    chapter_num: Option<&str>,
    volume_num: Option<&str>,
    cbz_path: &str,
) -> Result<()> {
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        r#"
        INSERT OR IGNORE INTO downloaded_chapters
            (chapter_id, manga_id, chapter_num, volume_num, cbz_path, downloaded_at)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6)
        "#,
    )
    .bind(chapter_id)
    .bind(manga_id)
    .bind(chapter_num)
    .bind(volume_num)
    .bind(cbz_path)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(())
}

#[derive(Debug)]
pub struct DownloadedChapter {
    pub chapter_id: String,
    pub manga_id: String,
    pub chapter_num: Option<String>,
    pub volume_num: Option<String>,
    pub cbz_path: String,
}

pub async fn all_downloaded_chapters(pool: &SqlitePool) -> Result<Vec<DownloadedChapter>> {
    let rows = sqlx::query_as::<_, (String, String, Option<String>, Option<String>, String)>(
        "SELECT chapter_id, manga_id, chapter_num, volume_num, cbz_path FROM downloaded_chapters",
    )
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(
        |(chapter_id, manga_id, chapter_num, volume_num, cbz_path)| DownloadedChapter {
            chapter_id,
            manga_id,
            chapter_num,
            volume_num,
            cbz_path,
        },
    )
    .collect();
    Ok(rows)
}

pub async fn update_cbz_path(pool: &SqlitePool, chapter_id: &str, cbz_path: &str) -> Result<()> {
    sqlx::query("UPDATE downloaded_chapters SET cbz_path = ?1 WHERE chapter_id = ?2")
        .bind(cbz_path)
        .bind(chapter_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn enqueue_chapter(
    pool: &SqlitePool,
    chapter_id: &str,
    manga_id: &str,
    priority: i64,
    chapter_num: Option<&str>,
    volume_num: Option<&str>,
) -> Result<()> {
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        r#"
        INSERT OR IGNORE INTO download_queue
            (chapter_id, manga_id, priority, chapter_num, volume_num, queued_at)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6)
        "#,
    )
    .bind(chapter_id)
    .bind(manga_id)
    .bind(priority)
    .bind(chapter_num)
    .bind(volume_num)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn next_queue_item(pool: &SqlitePool) -> Result<Option<QueueItem>> {
    let row = sqlx::query_as::<_, (i64, String, String, i64, i64)>(
        r#"
        SELECT id, chapter_id, manga_id, priority, attempts
        FROM download_queue
        WHERE attempts < 5
        ORDER BY priority DESC, queued_at ASC
        LIMIT 1
        "#,
    )
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|(id, chapter_id, manga_id, priority, attempts)| QueueItem {
        id,
        chapter_id,
        manga_id,
        priority,
        attempts,
    }))
}

pub async fn queue_item_failed(pool: &SqlitePool, id: i64, error: &str) -> Result<()> {
    sqlx::query(
        "UPDATE download_queue SET attempts = attempts + 1, last_error = ?1 WHERE id = ?2",
    )
    .bind(error)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn dequeue(pool: &SqlitePool, id: i64) -> Result<()> {
    sqlx::query("DELETE FROM download_queue WHERE id = ?1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn queue_size(pool: &SqlitePool) -> Result<i64> {
    let row: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM download_queue WHERE attempts < 5")
            .fetch_one(pool)
            .await?;
    Ok(row.0)
}

pub async fn nyaa_already_attempted(pool: &SqlitePool, manga_id: &str) -> Result<bool> {
    let row: (i64,) =
        sqlx::query_as("SELECT nyaa_attempted FROM catalog WHERE manga_id = ?1")
            .bind(manga_id)
            .fetch_one(pool)
            .await?;
    Ok(row.0 != 0)
}

pub async fn mark_nyaa_attempted(pool: &SqlitePool, manga_id: &str) -> Result<()> {
    sqlx::query("UPDATE catalog SET nyaa_attempted = 1 WHERE manga_id = ?1")
        .bind(manga_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn get_state(pool: &SqlitePool, key: &str) -> Result<Option<String>> {
    let row = sqlx::query_as::<_, (String,)>("SELECT value FROM state WHERE key = ?1")
        .bind(key)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(|(v,)| v))
}

pub async fn set_state(pool: &SqlitePool, key: &str, value: &str) -> Result<()> {
    sqlx::query(
        "INSERT INTO state (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind(key)
    .bind(value)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn catalog_title(pool: &SqlitePool, manga_id: &str) -> Result<Option<String>> {
    let row: Option<(String,)> = sqlx::query_as("SELECT title FROM catalog WHERE manga_id = ?1")
        .bind(manga_id)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(|(t,)| t))
}

/// Every chapter number we've either already downloaded or already queued
/// for this manga — used to tell whether a MangaDex-known volume is
/// genuinely missing (candidate for bulk backfill) or just has an isolated
/// gap the normal per-chapter pipeline will fill.
pub async fn all_known_chapter_nums(pool: &SqlitePool, manga_id: &str) -> Result<HashSet<String>> {
    let mut set = HashSet::new();
    let queued: Vec<(Option<String>,)> =
        sqlx::query_as("SELECT chapter_num FROM download_queue WHERE manga_id = ?1")
            .bind(manga_id)
            .fetch_all(pool)
            .await?;
    set.extend(queued.into_iter().filter_map(|(c,)| c));
    let downloaded: Vec<(Option<String>,)> =
        sqlx::query_as("SELECT chapter_num FROM downloaded_chapters WHERE manga_id = ?1")
            .bind(manga_id)
            .fetch_all(pool)
            .await?;
    set.extend(downloaded.into_iter().filter_map(|(c,)| c));
    Ok(set)
}

#[derive(Debug)]
pub struct Backfill {
    pub manga_id: String,
    pub status: String,
    pub volume_map: String,
}

#[derive(Debug)]
pub struct BackfillTorrent {
    pub volume: String,
    pub torrent_hash: String,
}

pub async fn backfill_exists(pool: &SqlitePool, manga_id: &str) -> Result<bool> {
    let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM backfills WHERE manga_id = ?1")
        .bind(manga_id)
        .fetch_one(pool)
        .await?;
    Ok(row.0 > 0)
}

pub async fn insert_backfill_needed(
    pool: &SqlitePool,
    manga_id: &str,
    volume_map_json: &str,
) -> Result<()> {
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        r#"
        INSERT OR IGNORE INTO backfills (manga_id, status, volume_map, created_at, updated_at)
        VALUES (?1, 'needed', ?2, ?3, ?3)
        "#,
    )
    .bind(manga_id)
    .bind(volume_map_json)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn backfills_by_status(pool: &SqlitePool, status: &str) -> Result<Vec<Backfill>> {
    let rows = sqlx::query_as::<_, (String, String, String)>(
        "SELECT manga_id, status, volume_map FROM backfills WHERE status = ?1",
    )
    .bind(status)
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|(manga_id, status, volume_map)| Backfill {
        manga_id,
        status,
        volume_map,
    })
    .collect();
    Ok(rows)
}

pub async fn set_backfill_status(pool: &SqlitePool, manga_id: &str, status: &str) -> Result<()> {
    sqlx::query("UPDATE backfills SET status = ?1, updated_at = ?2 WHERE manga_id = ?3")
        .bind(status)
        .bind(Utc::now().to_rfc3339())
        .bind(manga_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Records one torrent added to backfill a specific volume of a manga. A
/// manga's gap can now span several volumes, each fetched as its own
/// torrent — see `nyaa::match_volumes`.
pub async fn insert_backfill_torrent(
    pool: &SqlitePool,
    manga_id: &str,
    volume: &str,
    torrent_hash: &str,
) -> Result<()> {
    sqlx::query(
        r#"
        INSERT OR IGNORE INTO backfill_torrents (manga_id, volume, torrent_hash, added_at)
        VALUES (?1, ?2, ?3, ?4)
        "#,
    )
    .bind(manga_id)
    .bind(volume)
    .bind(torrent_hash)
    .bind(Utc::now().to_rfc3339())
    .execute(pool)
    .await?;
    Ok(())
}

/// Number of bulk-backfilled volumes already recorded for a manga (rows
/// inserted by `backfill::reconcile_one` with a synthetic `bulk:` chapter_id).
pub async fn bulk_downloaded_count(pool: &SqlitePool, manga_id: &str) -> Result<i64> {
    let row: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM downloaded_chapters WHERE manga_id = ?1 AND chapter_id LIKE 'bulk:%'",
    )
    .bind(manga_id)
    .fetch_one(pool)
    .await?;
    Ok(row.0)
}

pub async fn backfill_torrents_for_manga(
    pool: &SqlitePool,
    manga_id: &str,
) -> Result<Vec<BackfillTorrent>> {
    let rows = sqlx::query_as::<_, (String, String)>(
        "SELECT volume, torrent_hash FROM backfill_torrents WHERE manga_id = ?1",
    )
    .bind(manga_id)
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|(volume, torrent_hash)| BackfillTorrent { volume, torrent_hash })
    .collect();
    Ok(rows)
}

//! Historical-backlog backfill: some manga have far more chapters than
//! MangaDex hosts directly (e.g. it only mirrors the latest 3 of ~300
//! chapters across ~20 volumes). The normal per-chapter pipeline only ever
//! sees what MangaDex's feed returns, so those older volumes would just
//! never get downloaded — there's no "chapter failed 5 times" trigger for
//! chapters MangaDex doesn't list at all.
//!
//! This module closes that gap in three steps, run as their own scheduled
//! pass so they don't block the normal queueing flow:
//!   1. `check_and_trigger` — after queuing whatever MangaDex has for a
//!      newly-added manga, diff MangaDex's own volume/chapter aggregate
//!      against what we actually queued. Volumes with zero chapters present
//!      are recorded as `needed`.
//!   2. `run_pending_searches` — for `needed` rows, search nyaa and match
//!      each missing volume to its own torrent (nyaa releases manga one
//!      torrent per volume, not as a single "v01-20 complete" bundle — see
//!      `nyaa::match_volumes`), adding whichever ones it finds to
//!      qBittorrent under their own category so they can be found again
//!      later. A manga can end up with several torrents, one per volume.
//!   3. `reconcile` — once a volume's torrent finishes downloading, matches
//!      the resulting file to the chapter range MangaDex's aggregate
//!      reported for it (from step 1) and renames it into the library's
//!      naming scheme instead of leaving whatever name the uploader chose.
//!
//! Ongoing chapters continue through the normal MangaDex pipeline
//! unaffected — this only ever acts on volumes MangaDex has zero coverage
//! of.

use anyhow::{Context, Result};
use regex::Regex;
use sqlx::SqlitePool;
use std::path::Path;
use tracing::{info, warn};

use crate::config::Config;
use crate::db::queries;
use crate::library::{cbz, router};
use crate::mangadex::{aggregate, client::MangaDexClient};
use crate::nyaa::search as nyaa;
use crate::qbittorrent::client::QbittorrentClient;

/// Checks a newly-added manga's MangaDex aggregate against what was just
/// queued for it, and records a `needed` backfill if any volume has zero
/// chapters MangaDex hosts.
pub async fn check_and_trigger(
    client: &MangaDexClient,
    pool: &SqlitePool,
    manga_id: &str,
) -> Result<()> {
    if queries::backfill_exists(pool, manga_id).await? {
        return Ok(());
    }

    let known = queries::all_known_chapter_nums(pool, manga_id).await?;
    let gaps = aggregate::fetch_missing_volumes(client, manga_id, &known).await?;
    if gaps.is_empty() {
        return Ok(());
    }

    info!(
        manga_id,
        volumes = gaps.len(),
        "backfill needed: MangaDex hosts none of these volumes' chapters"
    );
    let volume_map = serde_json::to_string(&gaps).context("serialize volume_map")?;
    queries::insert_backfill_needed(pool, manga_id, &volume_map).await?;
    Ok(())
}

/// Caps how many pending manga get an nyaa/qBittorrent round-trip per call.
/// The backlog can be large (hundreds of manga after a catalog refresh) and
/// each round-trip involves a handful of network calls, so processing it
/// unbounded in one go risks a single slow tick running long enough that
/// nothing else makes progress in the meantime — better to chip away at it
/// every 15 minutes than to try to clear it all at once.
const SEARCH_BATCH_SIZE: usize = 150;

/// For every `needed` backfill (up to `SEARCH_BATCH_SIZE` per call), search
/// nyaa for a bulk release and add it to qBittorrent. Meant to be called
/// periodically rather than inline with queueing.
pub async fn run_pending_searches(pool: &SqlitePool, config: &Config, http: &reqwest::Client) {
    let pending = match queries::backfills_by_status(pool, "needed").await {
        Ok(rows) => rows,
        Err(e) => {
            warn!("backfill: list pending searches failed: {e:#}");
            return;
        }
    };
    if pending.is_empty() {
        return;
    }
    let total = pending.len();
    let pending: Vec<_> = pending.into_iter().take(SEARCH_BATCH_SIZE).collect();
    info!(
        batch = pending.len(),
        remaining = total,
        "backfill: searching nyaa for pending backfills"
    );

    let qbt = match QbittorrentClient::connect(
        &config.qbittorrent_url,
        &config.qbittorrent_username,
        &config.qbittorrent_password,
    )
    .await
    {
        Ok(c) => c,
        Err(e) => {
            warn!("backfill: qBittorrent connect failed: {e:#}");
            return;
        }
    };

    for row in pending {
        let title = match queries::catalog_title(pool, &row.manga_id).await {
            Ok(Some(t)) => t,
            Ok(None) => {
                warn!(manga_id = %row.manga_id, "backfill: no catalog title, skipping");
                continue;
            }
            Err(e) => {
                warn!("backfill: catalog title lookup failed: {e:#}");
                continue;
            }
        };
        let gaps: Vec<aggregate::VolumeGap> = match serde_json::from_str(&row.volume_map) {
            Ok(g) => g,
            Err(e) => {
                warn!("backfill: bad volume_map for {}: {e:#}", row.manga_id);
                continue;
            }
        };

        let results = match nyaa::search(http, &title).await {
            Ok(r) => r,
            Err(e) => {
                warn!(manga = %title, "backfill: nyaa search failed: {e:#}");
                continue;
            }
        };

        let volumes: Vec<String> = gaps.iter().map(|g| g.volume.clone()).collect();
        let matches = nyaa::match_volumes(&results, &title, &volumes);
        if matches.is_empty() {
            info!(manga = %title, "backfill: no per-volume release found on nyaa for any missing volume");
            let _ = queries::set_backfill_status(pool, &row.manga_id, "no_match").await;
            continue;
        }

        let safe_title = router::sanitize_title(&title);
        let save_path = Path::new(&config.nas_comics_path)
            .join(&safe_title)
            .to_string_lossy()
            .into_owned();

        let mut added = 0usize;
        for (volume, result) in &matches {
            let before = qbt.list_torrents("manga-backfill").await.unwrap_or_default();
            let before_hashes: std::collections::HashSet<_> =
                before.iter().map(|t| t.hash.clone()).collect();

            if let Err(e) = qbt
                .add_torrent_with_category(&result.torrent_url, &save_path, "manga-backfill")
                .await
            {
                warn!(manga = %title, volume, "backfill: qBittorrent add failed: {e:#}");
                continue;
            }

            let mut new_hash = None;
            for _ in 0..10 {
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                let after = match qbt.list_torrents("manga-backfill").await {
                    Ok(a) => a,
                    Err(_) => continue,
                };
                if let Some(t) = after.iter().find(|t| !before_hashes.contains(&t.hash)) {
                    new_hash = Some(t.hash.clone());
                    break;
                }
            }

            match new_hash {
                Some(hash) => {
                    info!(manga = %title, volume, torrent = %result.title, %hash, "backfill torrent added");
                    if let Err(e) =
                        queries::insert_backfill_torrent(pool, &row.manga_id, volume, &hash).await
                    {
                        warn!("backfill: record torrent failed: {e:#}");
                        continue;
                    }
                    added += 1;
                }
                None => {
                    warn!(manga = %title, volume, "backfill: torrent added but couldn't confirm its hash; will retry next pass");
                }
            }
        }

        let status = if added > 0 { "in_progress" } else { "no_match" };
        let _ = queries::set_backfill_status(pool, &row.manga_id, status).await;
        info!(
            manga = %title,
            matched = matches.len(),
            added,
            gaps = gaps.len(),
            "backfill: search pass complete"
        );
    }
}

/// For every `in_progress` backfill, checks whether any of its per-volume
/// torrents have finished downloading and, if so, renames the resulting
/// volume files into the library's naming scheme and records them. A manga
/// stays `in_progress` until every matched volume has been reconciled — any
/// still-downloading or since-removed torrents are simply retried next pass.
pub async fn reconcile(pool: &SqlitePool, config: &Config) {
    let pending = match queries::backfills_by_status(pool, "in_progress").await {
        Ok(rows) => rows,
        Err(e) => {
            warn!("backfill: list in_progress failed: {e:#}");
            return;
        }
    };
    if pending.is_empty() {
        return;
    }

    let qbt = match QbittorrentClient::connect(
        &config.qbittorrent_url,
        &config.qbittorrent_username,
        &config.qbittorrent_password,
    )
    .await
    {
        Ok(c) => c,
        Err(e) => {
            warn!("backfill: qBittorrent connect failed: {e:#}");
            return;
        }
    };
    let torrents = match qbt.list_torrents("manga-backfill").await {
        Ok(t) => t,
        Err(e) => {
            warn!("backfill: list torrents failed: {e:#}");
            return;
        }
    };

    for row in pending {
        let gaps: Vec<aggregate::VolumeGap> = match serde_json::from_str(&row.volume_map) {
            Ok(g) => g,
            Err(e) => {
                warn!("backfill: bad volume_map for {}: {e:#}", row.manga_id);
                continue;
            }
        };
        let manga_torrents = match queries::backfill_torrents_for_manga(pool, &row.manga_id).await
        {
            Ok(t) => t,
            Err(e) => {
                warn!("backfill: list torrents for {}: {e:#}", row.manga_id);
                continue;
            }
        };
        let any_complete = manga_torrents.iter().any(|bt| {
            torrents
                .iter()
                .find(|t| t.hash == bt.torrent_hash)
                .map(|t| t.progress >= 1.0)
                .unwrap_or(false)
        });
        if !any_complete {
            continue; // nothing new finished downloading yet
        }

        let title = match queries::catalog_title(pool, &row.manga_id).await {
            Ok(Some(t)) => t,
            _ => {
                warn!(manga_id = %row.manga_id, "backfill: no catalog title, skipping reconcile");
                continue;
            }
        };
        let safe_title = router::sanitize_title(&title);

        match reconcile_one(pool, config, &row.manga_id, &safe_title, &gaps).await {
            Ok(matched) => {
                let done_total = queries::bulk_downloaded_count(pool, &row.manga_id)
                    .await
                    .unwrap_or(0);
                info!(
                    manga = %safe_title,
                    matched,
                    done_total,
                    total = gaps.len(),
                    "backfill reconcile pass"
                );
                if done_total >= gaps.len() as i64 {
                    let _ = queries::set_backfill_status(pool, &row.manga_id, "done").await;
                }
            }
            Err(e) => {
                warn!(manga = %safe_title, "backfill: reconcile failed: {e:#}");
            }
        }
    }
}

fn volume_number_re() -> Regex {
    // No trailing `\b`: underscore counts as a word character, so e.g.
    // "v01_Title...cbz" (a real legacy naming shape) has no boundary between
    // "1" and "_" and would silently fail to match if one were required.
    Regex::new(r"(?i)\bv(?:ol(?:ume)?)?\.?\s*0*(\d+)").expect("valid regex")
}

async fn reconcile_one(
    pool: &SqlitePool,
    config: &Config,
    manga_id: &str,
    safe_title: &str,
    gaps: &[aggregate::VolumeGap],
) -> Result<usize> {
    let folder = Path::new(&config.nas_comics_path).join(safe_title);
    let already_chapter = Regex::new(r" c[0-9]{4}(\.[0-9]{3})? ?(\(v[0-9.]+\))?\.cbz$")?;
    let already_bulk = Regex::new(r" V[0-9]{4}(\.[0-9]{3})?-[0-9]{4}(\.[0-9]{3})? \(v[0-9.]+\)\.cbz$")?;
    let vol_re = volume_number_re();

    let mut matched = 0usize;
    let mut used_volumes = std::collections::HashSet::new();

    let entries = std::fs::read_dir(&folder)
        .with_context(|| format!("read manga folder {}", folder.display()))?;

    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        let Some(filename) = path.file_name().and_then(|f| f.to_str()) else {
            continue;
        };
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if ext != "cbz" && ext != "zip" {
            continue;
        }
        if already_chapter.is_match(filename) || already_bulk.is_match(filename) {
            continue;
        }
        let Some(caps) = vol_re.captures(filename) else {
            warn!(file = filename, "backfill: can't determine volume number, leaving as-is");
            continue;
        };
        let vol_num = &caps[1];
        let Some(gap) = gaps
            .iter()
            .find(|g| g.volume == vol_num && !used_volumes.contains(&g.volume))
        else {
            warn!(
                file = filename,
                volume = vol_num,
                "backfill: volume number doesn't match any known gap, leaving as-is"
            );
            continue;
        };

        let new_filename =
            cbz::bulk_volume_filename(safe_title, &gap.volume, &gap.first_chapter, &gap.last_chapter);
        let new_path = folder.join(&new_filename);
        if new_path.exists() {
            warn!(file = %new_path.display(), "backfill: destination already exists, skipping");
            continue;
        }

        std::fs::rename(&path, &new_path)
            .with_context(|| format!("rename {} -> {}", path.display(), new_path.display()))?;

        let synthetic_chapter_id = format!("bulk:{manga_id}:v{}", gap.volume);
        queries::mark_chapter_downloaded(
            pool,
            &synthetic_chapter_id,
            manga_id,
            Some(&gap.first_chapter),
            Some(&gap.volume),
            new_path.to_str().unwrap_or(""),
        )
        .await?;

        used_volumes.insert(gap.volume.clone());
        matched += 1;
    }

    Ok(matched)
}

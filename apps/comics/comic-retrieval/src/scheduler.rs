use anyhow::Result;
use chrono::Utc;
use sqlx::SqlitePool;
use std::sync::Arc;
use tokio_cron_scheduler::{Job, JobScheduler};
use tracing::{error, info};

use crate::backfill;
use crate::config::Config;
use crate::mangadex::catalog::{queue_daily_updates, queue_new_manga_chapters, refresh_catalog};
use crate::mangadex::client::MangaDexClient;

pub async fn setup(
    scheduler: &JobScheduler,
    client: Arc<MangaDexClient>,
    pool: SqlitePool,
    config: Arc<Config>,
) -> Result<()> {
    // Daily update at 03:00
    {
        let client = client.clone();
        let pool = pool.clone();
        scheduler
            .add(Job::new_async("0 0 3 * * *", move |_id, _lock| {
                let client = client.clone();
                let pool = pool.clone();
                Box::pin(async move {
                    if let Err(e) = run_daily(&client, &pool).await {
                        error!("daily update failed: {e:#}");
                    }
                })
            })?)
            .await?;
    }

    // Yearly catalog refresh on Jan 1 at 00:00
    {
        let client = client.clone();
        let pool = pool.clone();
        scheduler
            .add(Job::new_async("0 0 0 1 1 *", move |_id, _lock| {
                let client = client.clone();
                let pool = pool.clone();
                Box::pin(async move {
                    if let Err(e) = run_catalog_refresh(&client, &pool).await {
                        error!("catalog refresh failed: {e:#}");
                    }
                })
            })?)
            .await?;
    }

    // Backfill search + reconcile every 15 minutes
    {
        let pool = pool.clone();
        let config = config.clone();
        let http = reqwest::Client::builder()
            .user_agent("comic-retrieval/0.1")
            .timeout(std::time::Duration::from_secs(15))
            .build()?;
        scheduler
            .add(Job::new_async("0 */15 * * * *", move |_id, _lock| {
                let pool = pool.clone();
                let config = config.clone();
                let http = http.clone();
                Box::pin(async move {
                    info!("backfill: tick fired");
                    // Belt-and-suspenders: every individual network call in
                    // this path has its own timeout, but an overall deadline
                    // means one tick hanging can never block every tick after
                    // it — the 15-minute schedule keeps making progress.
                    let deadline = std::time::Duration::from_secs(600);
                    if tokio::time::timeout(deadline, backfill::run_pending_searches(&pool, &config, &http))
                        .await
                        .is_err()
                    {
                        error!("backfill: run_pending_searches exceeded {deadline:?}, aborting this tick");
                        return;
                    }
                    if tokio::time::timeout(deadline, backfill::reconcile(&pool, &config))
                        .await
                        .is_err()
                    {
                        error!("backfill: reconcile exceeded {deadline:?}, aborting this tick");
                    }
                })
            })?)
            .await?;
    }

    info!("scheduler: daily=03:00, yearly=Jan-1-00:00, backfill=every-15min");
    Ok(())
}

pub async fn run_daily(client: &MangaDexClient, pool: &SqlitePool) -> Result<()> {
    // MangaDex's updatedAtSince param requires exactly YYYY-MM-DDTHH:MM:SS —
    // no fractional seconds, no timezone offset (to_rfc3339() produces both
    // and gets rejected with a 400 validation_exception on every request).
    const MANGADEX_TS_FMT: &str = "%Y-%m-%dT%H:%M:%S";

    let stored = crate::db::queries::get_state(pool, "daily_last_run").await?;
    // Normalize at read time too: a value stored by a previous build (RFC3339,
    // with fractional seconds + offset) would otherwise keep failing forever
    // even after this fix, since it's read back as-is on every run.
    let since = match stored.as_deref().map(chrono::DateTime::parse_from_rfc3339) {
        Some(Ok(dt)) => dt.format(MANGADEX_TS_FMT).to_string(),
        Some(Err(_)) | None => stored.unwrap_or_else(|| {
            // default: 25 hours ago
            (Utc::now() - chrono::TimeDelta::hours(25))
                .format(MANGADEX_TS_FMT)
                .to_string()
        }),
    };

    info!("daily update: checking chapters since {since}");
    let queued = queue_daily_updates(client, pool, &since).await?;
    info!("daily update: queued {queued} new chapters");

    crate::db::queries::set_state(
        pool,
        "daily_last_run",
        &Utc::now().format(MANGADEX_TS_FMT).to_string(),
    )
    .await?;
    Ok(())
}

pub async fn run_catalog_refresh(client: &MangaDexClient, pool: &SqlitePool) -> Result<()> {
    let added = refresh_catalog(client, pool).await?;
    info!("catalog refresh: {added} manga in catalog");

    // queue chapters for any manga that have never had a download
    let catalog = crate::db::queries::all_catalog(pool).await?;
    let mut total_queued = 0usize;
    for entry in &catalog {
        if !crate::db::queries::manga_has_any_download(pool, &entry.manga_id).await? {
            match queue_new_manga_chapters(client, pool, &entry.manga_id, 0).await {
                Ok(n) => total_queued += n,
                Err(e) => error!("queue chapters for {}: {e:#}", entry.manga_id),
            }
        }

        // Deliberately NOT gated behind "has never had a download": a manga
        // can have a single straggler chapter downloaded years ago (a nyaa
        // single-chapter fallback hit, or a MangaDex quirk) while MangaDex
        // reports hundreds more chapters exist that were never queued —
        // e.g. real cases found in this library: Bleach had exactly one
        // chapter (336) downloaded while MangaDex reports 686 exist;
        // InuYasha had only chapter 1 of 559. Gating this check on zero
        // downloads would permanently exclude both from ever being
        // reconsidered for backfill. check_and_trigger's own
        // `backfill_exists` guard already makes repeat calls for
        // already-checked manga a cheap no-op.
        if let Err(e) = backfill::check_and_trigger(client, pool, &entry.manga_id).await {
            error!("backfill check for {}: {e:#}", entry.manga_id);
        }
    }
    info!("catalog refresh: queued {total_queued} chapters for new manga");
    Ok(())
}

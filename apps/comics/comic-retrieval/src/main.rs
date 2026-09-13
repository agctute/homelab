use comic_retrieval::{config, db, mangadex, scheduler, worker};

use anyhow::Result;
use std::sync::Arc;
use tokio_cron_scheduler::JobScheduler;
use tracing::info;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::registry()
        .with(EnvFilter::from_env("LOG_LEVEL").add_directive("info".parse().unwrap()))
        .with(fmt::layer())
        .init();

    let config = Arc::new(config::Config::from_env()?);
    info!("connecting to database at {}", config.db_path);
    let pool = db::connect(&config.db_path).await?;

    info!("authenticating with MangaDex");
    let client = Arc::new(mangadex::client::MangaDexClient::new(config.clone()).await?);
    info!("MangaDex auth OK");

    // optional on-start triggers (useful for first-run / debugging) — run in
    // the background rather than blocking startup. The catalog can hold
    // thousands of entries competing for the same rate-limited MangaDex
    // client as the download worker and the scheduled jobs below, so this
    // can take hours; nothing else (worker, daily/yearly cron, backfill
    // search/reconcile) should have to wait on it to start.
    if config.run_catalog_refresh_on_start || config.run_daily_update_on_start {
        let startup_client = client.clone();
        let startup_pool = pool.clone();
        let run_catalog_refresh = config.run_catalog_refresh_on_start;
        let run_daily_update = config.run_daily_update_on_start;
        tokio::spawn(async move {
            if run_catalog_refresh {
                info!("RUN_CATALOG_REFRESH_ON_START=true, running now");
                if let Err(e) = scheduler::run_catalog_refresh(&startup_client, &startup_pool).await {
                    tracing::error!("startup catalog refresh failed: {e:#}");
                }
            }
            if run_daily_update {
                info!("RUN_DAILY_UPDATE_ON_START=true, running now");
                if let Err(e) = scheduler::run_daily(&startup_client, &startup_pool).await {
                    tracing::error!("startup daily update failed: {e:#}");
                }
            }
        });
    }

    // start background download worker
    let worker_pool = pool.clone();
    let worker_client = client.clone();
    let worker_config = config.clone();
    tokio::spawn(async move {
        worker::run(worker_client, worker_pool, worker_config).await;
    });

    // set up cron scheduler
    let mut sched = JobScheduler::new().await?;
    scheduler::setup(&sched, client, pool, config).await?;
    sched.start().await?;

    info!("comic-retrieval running");
    tokio::signal::ctrl_c().await?;
    info!("shutting down");
    sched.shutdown().await?;
    Ok(())
}

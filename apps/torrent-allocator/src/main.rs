mod apps;
mod classifier;
mod config;
mod db;
mod format;
mod fsutil;
mod metadata;
mod qbittorrent;
mod watcher;

use anyhow::Result;
use tracing::info;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

use classifier::openai::OpenAiClassifier;
use classifier::Classifier;
use metadata::google_books::GoogleBooksClient;
use qbittorrent::client::QbittorrentClient;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::registry()
        .with(EnvFilter::from_env("LOG_LEVEL").add_directive("info".parse().unwrap()))
        .with(fmt::layer())
        .init();

    let config = config::Config::from_env()?;

    info!("connecting to database at {}", config.db_path);
    let pool = db::connect(&config.db_path).await?;

    info!("connecting to qBittorrent at {}", config.qbittorrent_url);
    let qbt = QbittorrentClient::connect(
        &config.qbittorrent_url,
        &config.qbittorrent_username,
        &config.qbittorrent_password,
    )
    .await?;

    let classifier: Box<dyn Classifier> = Box::new(OpenAiClassifier::new(
        config.openai_api_key.clone(),
        config.openai_model.clone(),
    ));
    let book_metadata_client = GoogleBooksClient::new(config.google_books_api_key.clone());

    let apps = apps::registry();
    info!(
        apps = ?apps.iter().map(|a| a.id()).collect::<Vec<_>>(),
        "registered app handlers"
    );

    let watcher_config = config.clone();
    tokio::spawn(async move {
        watcher::run(
            qbt,
            classifier,
            book_metadata_client,
            apps,
            pool,
            watcher_config,
        )
        .await;
    });

    info!("torrent-allocator running");
    tokio::signal::ctrl_c().await?;
    info!("shutting down");
    Ok(())
}

use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use sqlx::SqlitePool;
use tracing::{error, info, warn};

use crate::apps::{AppHandler, CompletedTorrent};
use crate::classifier::{AppInfo, Classification, ClassificationInput, Classifier};
use crate::config::Config;
use crate::db::queries;
use crate::fsutil;
use crate::metadata::{self, google_books::GoogleBooksClient};
use crate::qbittorrent::client::QbittorrentClient;

pub async fn run(
    qbt: QbittorrentClient,
    classifier: Box<dyn Classifier>,
    book_metadata_client: GoogleBooksClient,
    apps: Vec<Box<dyn AppHandler>>,
    pool: SqlitePool,
    config: Config,
) {
    info!(
        interval_secs = config.poll_interval_secs,
        "torrent watcher started"
    );

    loop {
        if let Err(e) = tick(
            &qbt,
            classifier.as_ref(),
            &book_metadata_client,
            &apps,
            &pool,
            &config,
        )
        .await
        {
            error!("watcher tick failed: {e:#}");
        }
        tokio::time::sleep(Duration::from_secs(config.poll_interval_secs)).await;
    }
}

async fn tick(
    qbt: &QbittorrentClient,
    classifier: &dyn Classifier,
    book_metadata_client: &GoogleBooksClient,
    apps: &[Box<dyn AppHandler>],
    pool: &SqlitePool,
    config: &Config,
) -> Result<()> {
    let candidates: Vec<AppInfo> = apps
        .iter()
        .map(|a| AppInfo {
            id: a.id(),
            description: a.description(),
        })
        .collect();

    for t in qbt.list_completed().await? {
        if queries::is_processed(pool, &t.hash).await? {
            continue;
        }

        let category = if t.category.is_empty() {
            None
        } else {
            Some(t.category.as_str())
        };

        let content_path = PathBuf::from(&t.content_path);
        let file_extensions = fsutil::list_extensions(&content_path).await;

        let search_title = metadata::guess_search_title(&t.name);
        let book_metadata = book_metadata_client.search(&search_title).await;

        let classification: Classification = match classifier
            .classify(
                ClassificationInput {
                    torrent_name: &t.name,
                    category,
                    file_extensions: &file_extensions,
                    book_metadata: book_metadata.as_ref(),
                },
                &candidates,
            )
            .await
        {
            Ok(c) => c,
            Err(e) => {
                warn!(torrent = %t.name, "classification failed: {e:#}, will retry next poll");
                continue;
            }
        };

        info!(
            torrent = %t.name,
            app = %classification.app_id,
            title = ?classification.title,
            language = ?classification.language,
            ?file_extensions,
            content_path = %content_path.display(),
            "classified torrent"
        );

        // English only — anything else gets recorded and skipped rather than
        // retried forever.
        if let Some(lang) = &classification.language {
            if !is_english(lang) {
                warn!(torrent = %t.name, language = %lang, "non-English content, skipping");
                queries::mark_processed(
                    pool,
                    &t.hash,
                    &t.name,
                    &classification.app_id,
                    "skipped-non-english",
                    Some(lang.as_str()),
                )
                .await?;
                continue;
            }
        }

        let completed = CompletedTorrent {
            hash: t.hash.clone(),
            name: t.name.clone(),
            category: category.map(str::to_string),
            content_path,
            save_path: t.save_path.clone(),
        };

        match apps.iter().find(|a| a.id() == classification.app_id) {
            Some(handler) => match handler.handle(&completed, &classification, config).await {
                Ok(()) => {
                    queries::mark_processed(
                        pool,
                        &t.hash,
                        &t.name,
                        &classification.app_id,
                        "handled",
                        None,
                    )
                    .await?;
                }
                Err(e) => {
                    warn!(
                        torrent = %t.name,
                        app = %classification.app_id,
                        "handler failed: {e:#}, will retry next poll"
                    );
                }
            },
            None => {
                warn!(
                    torrent = %t.name,
                    app = %classification.app_id,
                    "no handler registered for this app, marking unsorted"
                );
                queries::mark_processed(
                    pool,
                    &t.hash,
                    &t.name,
                    "unknown",
                    "unsorted",
                    classification.reasoning.as_deref(),
                )
                .await?;
            }
        }
    }

    Ok(())
}

fn is_english(language: &str) -> bool {
    matches!(
        language.trim().to_ascii_lowercase().as_str(),
        "en" | "eng" | "english"
    )
}

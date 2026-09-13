//! One-off cleanup: removes manga from the catalog and NAS entirely, by
//! manga_id. Used to purge content that shouldn't have been auto-added in
//! the first place — e.g. doujinshi/fan-colored entries that predate the
//! content-rating and tag filters in `mangadex::catalog::build_list_url`.
//!
//! Reads manga_ids (one per line) from the path in MANGA_IDS_FILE (default
//! /tmp/purge_ids.txt). For each: deletes its NAS folder, its rows in
//! downloaded_chapters/download_queue/backfills/backfill_torrents, and its
//! catalog row.
//!
//! Defaults to a dry run. Pass `--apply` to actually delete anything.

use anyhow::Result;
use comic_retrieval::db;
use comic_retrieval::library::router;
use std::path::Path;

#[tokio::main]
async fn main() -> Result<()> {
    let apply = std::env::args().any(|a| a == "--apply");
    let db_path = std::env::var("DB_PATH").unwrap_or_else(|_| "/comics/state.db".to_string());
    let nas_path = std::env::var("NAS_COMICS_PATH").unwrap_or_else(|_| "/books/manga".to_string());
    let ids_file = std::env::var("MANGA_IDS_FILE").unwrap_or_else(|_| "/tmp/purge_ids.txt".to_string());

    let ids_content = std::fs::read_to_string(&ids_file)
        .map_err(|e| anyhow::anyhow!("reading {ids_file}: {e}"))?;
    let ids: Vec<&str> = ids_content.lines().map(|l| l.trim()).filter(|l| !l.is_empty()).collect();
    println!("connecting to {db_path}, {} manga_ids to purge from {ids_file}", ids.len());

    let pool = db::connect(&db_path).await?;

    let mut folders_deleted = 0usize;
    let mut catalog_rows_deleted = 0usize;
    let mut not_in_catalog = 0usize;

    for manga_id in ids {
        let title: Option<(String,)> =
            sqlx::query_as("SELECT title FROM catalog WHERE manga_id = ?1")
                .bind(manga_id)
                .fetch_optional(&pool)
                .await?;
        let Some((title,)) = title else {
            println!("{manga_id}: not in catalog, skipping");
            not_in_catalog += 1;
            continue;
        };
        let safe_title = router::sanitize_title(&title);
        let folder = Path::new(&nas_path).join(&safe_title);

        let chapter_count: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM downloaded_chapters WHERE manga_id = ?1")
                .bind(manga_id)
                .fetch_one(&pool)
                .await?;
        let queue_count: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM download_queue WHERE manga_id = ?1")
                .bind(manga_id)
                .fetch_one(&pool)
                .await?;

        println!(
            "{title} ({manga_id}): folder={} chapters={} queued={}",
            folder.display(),
            chapter_count.0,
            queue_count.0
        );

        if apply {
            if folder.exists() {
                std::fs::remove_dir_all(&folder)?;
                folders_deleted += 1;
            }
            sqlx::query("DELETE FROM downloaded_chapters WHERE manga_id = ?1")
                .bind(manga_id)
                .execute(&pool)
                .await?;
            sqlx::query("DELETE FROM download_queue WHERE manga_id = ?1")
                .bind(manga_id)
                .execute(&pool)
                .await?;
            sqlx::query("DELETE FROM backfill_torrents WHERE manga_id = ?1")
                .bind(manga_id)
                .execute(&pool)
                .await?;
            sqlx::query("DELETE FROM backfills WHERE manga_id = ?1")
                .bind(manga_id)
                .execute(&pool)
                .await?;
            sqlx::query("DELETE FROM catalog WHERE manga_id = ?1")
                .bind(manga_id)
                .execute(&pool)
                .await?;
            catalog_rows_deleted += 1;
        } else if folder.exists() {
            folders_deleted += 1;
        }
    }

    println!(
        "\n{}: {catalog_rows_deleted} catalog rows {}, {folders_deleted} folders {}, {not_in_catalog} not found",
        if apply { "applied" } else { "dry run (pass --apply to execute)" },
        if apply { "deleted" } else { "would delete" },
        if apply { "deleted" } else { "would delete" },
    );

    Ok(())
}

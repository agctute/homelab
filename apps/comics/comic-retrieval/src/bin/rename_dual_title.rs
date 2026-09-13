//! One-off migration: renames each series folder to include both its
//! English and (romanized-preferred) Japanese names —
//! "{English} ({Japanese})" — for entries where both are available and
//! differ, and records the Japanese name in every existing file's
//! ComicInfo.xml (<AlternateSeries>).
//!
//! Individual filenames are deliberately left untouched — only the catalog
//! title and folder name change. Since new chapters are always saved under
//! `catalog.title`, chapters downloaded *after* this migration for a
//! renamed series will naturally use the new dual name in their own
//! filename; only pre-existing files keep their current single-name form.
//!
//! Fetches fresh title/altTitles data from MangaDex in batches of 100 via
//! GET /manga?ids[]=... (public endpoint, no auth needed).
//!
//! Defaults to a dry run. Pass `--apply` to actually rename folders and
//! rewrite ComicInfo.xml.

use anyhow::{Context, Result};
use comic_retrieval::db;
use comic_retrieval::library::{cbz, router};
use comic_retrieval::mangadex::models::Manga;
use serde::Deserialize;
use sqlx::SqlitePool;
use std::collections::HashMap;
use std::path::Path;

#[derive(Deserialize)]
struct MangaListResponse {
    data: Vec<Manga>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let apply = std::env::args().any(|a| a == "--apply");
    let db_path = std::env::var("DB_PATH").unwrap_or_else(|_| "/comics/state.db".to_string());
    let nas_path = std::env::var("NAS_COMICS_PATH").unwrap_or_else(|_| "/books/manga".to_string());

    let pool = db::connect(&db_path).await?;
    let rows: Vec<(String, String)> = sqlx::query_as("SELECT manga_id, title FROM catalog")
        .fetch_all(&pool)
        .await?;
    println!("{} catalog entries", rows.len());

    let http = reqwest::Client::builder()
        .user_agent("rename-dual-title/1.0")
        .timeout(std::time::Duration::from_secs(20))
        .build()?;

    let mut renamed = 0usize;
    let mut comicinfo_would_change = 0usize;
    let mut unchanged = 0usize;
    let mut no_folder = 0usize;
    let mut collisions = 0usize;
    let mut not_on_mangadex = 0usize;
    let mut failures = 0usize;

    for chunk in rows.chunks(100) {
        let ids_param: String =
            chunk.iter().map(|(id, _)| format!("ids[]={id}")).collect::<Vec<_>>().join("&");
        let url = format!("https://api.mangadex.org/manga?{ids_param}&limit=100");

        let resp = match http.get(&url).send().await {
            Ok(r) => r,
            Err(e) => {
                eprintln!("batch request failed: {e:#}");
                continue;
            }
        };
        let parsed: Result<MangaListResponse, _> = resp.json().await;
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        let Ok(parsed) = parsed else {
            eprintln!("batch parse failed");
            continue;
        };
        let by_id: HashMap<String, Manga> =
            parsed.data.into_iter().map(|m| (m.id.clone(), m)).collect();

        for (manga_id, current_title) in chunk {
            let Some(manga) = by_id.get(manga_id) else {
                not_on_mangadex += 1;
                continue;
            };
            let dual = manga.dual_title();
            if &dual.combined == current_title {
                unchanged += 1;
                continue;
            }

            let old_safe = router::sanitize_title(current_title);
            let new_safe = router::sanitize_title(&dual.combined);
            let old_folder = Path::new(&nas_path).join(&old_safe);
            let new_folder = Path::new(&nas_path).join(&new_safe);

            if !old_folder.exists() {
                println!("{manga_id}: {current_title:?} -> {:?} (no folder on disk, catalog title only)", dual.combined);
                no_folder += 1;
                if apply {
                    if let Err(e) = sqlx::query("UPDATE catalog SET title = ?1 WHERE manga_id = ?2")
                        .bind(&dual.combined)
                        .bind(manga_id)
                        .execute(&pool)
                        .await
                    {
                        eprintln!("{manga_id}: catalog title update failed, skipping: {e:#}");
                        failures += 1;
                    }
                }
                continue;
            }
            if new_folder.exists() {
                eprintln!(
                    "{manga_id}: collision, {} already exists, skipping",
                    new_folder.display()
                );
                collisions += 1;
                continue;
            }

            println!("{} -> {}", old_folder.display(), new_folder.display());

            // A single entry failing (unexpected filesystem error, DB
            // hiccup, ...) shouldn't take the whole batch down with it —
            // log it and move on to the next manga instead of aborting.
            let folder_for_comicinfo = if apply {
                match rename_one(&pool, manga_id, &old_folder, &new_folder, &dual.combined).await {
                    Ok(()) => new_folder,
                    Err(e) => {
                        eprintln!("{manga_id}: rename failed, skipping: {e:#}");
                        failures += 1;
                        continue;
                    }
                }
            } else {
                old_folder
            };
            renamed += 1;

            if let Some(japanese) = &dual.japanese {
                let Ok(entries) = std::fs::read_dir(&folder_for_comicinfo) else { continue };
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.extension().and_then(|e| e.to_str()) != Some("cbz") {
                        continue;
                    }
                    match cbz::set_alternate_series(&path, japanese, apply) {
                        Ok(true) => comicinfo_would_change += 1,
                        Ok(false) => {}
                        Err(e) => eprintln!("  ComicInfo update failed for {}: {e:#}", path.display()),
                    }
                }
            }
        }
    }

    println!(
        "\n{}: {renamed} folders {}, {comicinfo_would_change} files' ComicInfo {}, \
         {unchanged} unchanged, {no_folder} not yet downloaded (catalog title only), \
         {collisions} collisions, {not_on_mangadex} not found on MangaDex, {failures} failed (skipped, logged above)",
        if apply { "applied" } else { "dry run (pass --apply to execute)" },
        if apply { "renamed" } else { "would rename" },
        if apply { "updated" } else { "would update" },
    );

    Ok(())
}

/// Renames one manga's folder, updates its chapters' cbz_path, and updates
/// its catalog title — as a single fallible unit so the caller can log and
/// skip to the next manga on failure instead of aborting the whole batch.
async fn rename_one(
    pool: &SqlitePool,
    manga_id: &str,
    old_folder: &Path,
    new_folder: &Path,
    new_title: &str,
) -> Result<()> {
    std::fs::rename(old_folder, new_folder)
        .with_context(|| format!("rename {} -> {}", old_folder.display(), new_folder.display()))?;
    update_cbz_paths(pool, manga_id, new_folder).await?;
    sqlx::query("UPDATE catalog SET title = ?1 WHERE manga_id = ?2")
        .bind(new_title)
        .bind(manga_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Updates cbz_path for every downloaded chapter of `manga_id` to reflect
/// its folder having moved — the filename itself is unchanged, only the
/// containing directory.
async fn update_cbz_paths(pool: &SqlitePool, manga_id: &str, new_folder: &Path) -> Result<()> {
    let chapters: Vec<(String, String)> =
        sqlx::query_as("SELECT chapter_id, cbz_path FROM downloaded_chapters WHERE manga_id = ?1")
            .bind(manga_id)
            .fetch_all(pool)
            .await?;
    for (chapter_id, old_path) in chapters {
        let Some(filename) = Path::new(&old_path).file_name().and_then(|f| f.to_str()) else {
            continue;
        };
        let new_path = new_folder.join(filename);
        sqlx::query("UPDATE downloaded_chapters SET cbz_path = ?1 WHERE chapter_id = ?2")
            .bind(new_path.to_str().unwrap_or(""))
            .bind(chapter_id)
            .execute(pool)
            .await?;
    }
    Ok(())
}

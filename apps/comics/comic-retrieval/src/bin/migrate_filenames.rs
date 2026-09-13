//! One-off migration: rewrites existing CBZ filenames on the NAS to the
//! chapter-first naming scheme (see `library::cbz::cbz_filename`), and
//! updates `downloaded_chapters.cbz_path` to match.
//!
//! Old files sometimes sorted the latest chapter first because volume-tagged
//! and volume-less chapters produced differently-shaped filenames. This
//! rewrites every existing file to the corrected format so the whole
//! library — not just newly downloaded chapters — sorts correctly.
//!
//! Also repairs `downloaded_chapters` rows left stale by duplicate scanlation
//! releases: if two chapter_ids share the same (title, volume, chapter), they
//! compute to the same filename and — both before and after this rewrite —
//! only one file can physically exist at that path. When the file already
//! exists at the computed destination:
//!   - if this row's own on-disk file is gone (a sibling row already moved
//!     the shared file there), just repoint this row's `cbz_path` to match
//!     reality — no file operation needed.
//!   - if this row's file is still separately present (genuinely distinct,
//!     not-yet-deduplicated content), rename it aside with a chapter_id
//!     suffix instead of silently overwriting anything.
//!
//! Defaults to a dry run (prints planned changes, touches nothing).
//! Pass `--apply` to actually rename files on disk and update the DB.

use anyhow::{Context, Result};
use comic_retrieval::db;
use comic_retrieval::library::cbz;
use std::path::{Path, PathBuf};

#[tokio::main]
async fn main() -> Result<()> {
    let apply = std::env::args().any(|a| a == "--apply");

    let db_path = std::env::var("DB_PATH").unwrap_or_else(|_| "/comics/state.db".to_string());
    println!("connecting to {db_path}");
    let pool = db::connect(&db_path).await?;

    let chapters = db::queries::all_downloaded_chapters(&pool).await?;
    println!("{} downloaded chapters on record", chapters.len());

    let mut renamed = 0;
    let mut already_ok = 0;
    let mut missing = 0;
    let mut repointed = 0;
    let mut disambiguated = 0;

    for ch in chapters {
        let old_path = Path::new(&ch.cbz_path);
        let Some(parent) = old_path.parent() else {
            eprintln!("skip {}: no parent dir in {}", ch.chapter_id, ch.cbz_path);
            continue;
        };
        let Some(title) = parent.file_name().and_then(|n| n.to_str()) else {
            eprintln!("skip {}: can't derive title from {}", ch.chapter_id, ch.cbz_path);
            continue;
        };

        let new_filename = cbz::cbz_filename(title, ch.volume_num.as_deref(), ch.chapter_num.as_deref());
        let new_path = parent.join(&new_filename);

        if new_path == old_path {
            already_ok += 1;
            continue;
        }

        if new_path.exists() {
            if old_path.exists() {
                let dest = disambiguate(&new_path, &ch.chapter_id);
                if dest.exists() {
                    eprintln!(
                        "can't disambiguate, skipping: {} (both {} and {} already exist)",
                        old_path.display(),
                        new_path.display(),
                        dest.display()
                    );
                    continue;
                }
                println!(
                    "duplicate content, disambiguating: {} -> {}",
                    old_path.display(),
                    dest.display()
                );
                if apply {
                    std::fs::rename(old_path, &dest).with_context(|| {
                        format!("rename {} -> {}", old_path.display(), dest.display())
                    })?;
                    db::queries::update_cbz_path(&pool, &ch.chapter_id, dest.to_str().unwrap_or(""))
                        .await?;
                }
                disambiguated += 1;
            } else {
                println!(
                    "repointing stale db row: {} -> {} (file already there)",
                    ch.cbz_path,
                    new_path.display()
                );
                if apply {
                    db::queries::update_cbz_path(&pool, &ch.chapter_id, new_path.to_str().unwrap_or(""))
                        .await?;
                }
                repointed += 1;
            }
            continue;
        }

        if !old_path.exists() {
            eprintln!(
                "missing on disk, skipping: {} (db chapter_id={})",
                old_path.display(),
                ch.chapter_id
            );
            missing += 1;
            continue;
        }

        println!("{} -> {}", old_path.display(), new_path.display());

        if apply {
            std::fs::rename(old_path, &new_path)
                .with_context(|| format!("rename {} -> {}", old_path.display(), new_path.display()))?;
            db::queries::update_cbz_path(&pool, &ch.chapter_id, new_path.to_str().unwrap_or(""))
                .await?;
        }
        renamed += 1;
    }

    println!(
        "\n{}: {renamed} renamed, {already_ok} already correct, {repointed} stale rows repointed, \
         {disambiguated} duplicates disambiguated, {missing} missing on disk",
        if apply { "applied" } else { "dry run (pass --apply to execute)" }
    );

    Ok(())
}

/// Appends a short chapter_id-derived suffix to a filename so a genuinely
/// distinct duplicate release doesn't overwrite the file already at `path`.
fn disambiguate(path: &Path, chapter_id: &str) -> PathBuf {
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("chapter");
    let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("cbz");
    let short_id: String = chapter_id.chars().filter(|c| *c != '-').take(8).collect();
    path.with_file_name(format!("{stem} [{short_id}].{ext}"))
}

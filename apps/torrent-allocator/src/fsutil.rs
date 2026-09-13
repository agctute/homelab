use anyhow::{Context, Result};
use std::path::Path;

/// Filesystem-safe version of a title: strips characters that are awkward
/// on most filesystems and collapses whitespace.
pub fn sanitize(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => ' ',
            other => other,
        })
        .collect();
    cleaned.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub async fn ensure_dir(path: &Path) -> Result<()> {
    tokio::fs::create_dir_all(path)
        .await
        .with_context(|| format!("create directory {}", path.display()))
}

/// Lists the distinct file extensions present in a torrent's downloaded
/// content — a single file's own extension, or a shallow (non-recursive)
/// scan of a directory's immediate children. Used to give the classifier a
/// hard signal about actual file format (e.g. `.m4b` = audio) rather than
/// relying on it to infer format from the torrent's name alone, which often
/// doesn't carry a extension at all (e.g. a torrent named after a book
/// title with no file extension, containing a single `.m4b` inside).
pub async fn list_extensions(path: &Path) -> Vec<String> {
    if path.is_file() {
        return extension_of(path).into_iter().collect();
    }

    let mut entries = match tokio::fs::read_dir(path).await {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    let mut exts = Vec::new();
    while let Ok(Some(entry)) = entries.next_entry().await {
        let p = entry.path();
        if p.is_file() {
            if let Some(ext) = extension_of(&p) {
                if !exts.contains(&ext) {
                    exts.push(ext);
                }
            }
        }
    }
    exts
}

fn extension_of(path: &Path) -> Option<String> {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
}

/// Copies a file into the library — never deletes the source. qBittorrent
/// keeps seeding straight out of `/torrents` indefinitely, so anything
/// touched here must still exist there afterward; a move (rename or
/// copy+delete) would pull the file out from under an active seed.
pub async fn copy_file(src: &Path, dest: &Path) -> Result<()> {
    if let Some(parent) = dest.parent() {
        ensure_dir(parent).await?;
    }
    tokio::fs::copy(src, dest)
        .await
        .with_context(|| format!("copy {} to {}", src.display(), dest.display()))?;
    Ok(())
}

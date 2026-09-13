//! One-off cleanup: renames CBZ files that came in through the nyaa.si/
//! qBittorrent fallback (see worker::try_nyaa_fallback) and so never went
//! through `cbz::cbz_filename` — they kept whatever name the torrent
//! uploader gave them instead.
//!
//! These files have no row in the SQLite DB at all (the fallback never calls
//! `mark_chapter_downloaded`), so this walks the filesystem directly rather
//! than the DB, unlike `migrate_filenames`.
//!
//! Only renames filenames a chapter/volume number can be confidently
//! extracted from — plain per-chapter releases like `Foo c014 (...).cbz` or
//! `Foo 077 (2022) (...).cbz`. Deliberately leaves alone anything that isn't
//! clearly a single chapter: whole-volume releases (`Foo v01 (...).cbz`),
//! chapter ranges (`Foo c57-58.cbz`), and one-shots/unrecognized shapes —
//! those aren't safely representable as `cNNNN` without misrepresenting what
//! they are, and get logged as skipped rather than guessed at.
//!
//! Defaults to a dry run. Pass `--apply` to actually rename files.

use comic_retrieval::library::cbz;
use regex::Regex;
use std::path::{Path, PathBuf};

fn main() -> anyhow::Result<()> {
    let apply = std::env::args().any(|a| a == "--apply");
    let root = std::env::var("NAS_COMICS_PATH").unwrap_or_else(|_| "/books/manga".to_string());

    println!("scanning {root}");

    let already_canonical =
        Regex::new(r"(?i) c[0-9]{4}(\.[0-9]{3})? ?(\(v[0-9.]+\))?\.cbz$").expect("valid regex");
    // `regex` has no look-around support, so the "must be immediately
    // followed by whitespace/paren/end" boundary is checked manually after
    // matching (see `matches_boundary` below) instead of via a lookahead.
    let chapter_prefixed =
        Regex::new(r"(?:^|\s)c(\d+(?:\.\d+){0,2}[a-zA-Z]?)").expect("valid regex");
    let bare_numbered = Regex::new(r"(?:^|\s)(\d{1,4}(?:\.\d+)?)\s*\((?:19|20)\d{2}\)")
        .expect("valid regex");
    let volume_re = Regex::new(r"\(v(\d+(?:\.\d+)?)\)").expect("valid regex");

    let mut files = Vec::new();
    walk(Path::new(&root), &mut files)?;

    let mut renamed = 0;
    let mut collisions = 0;
    let mut skipped = 0;

    for path in files {
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let Some(filename) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        if already_canonical.is_match(filename) {
            continue;
        }
        let Some(parent) = path.parent() else { continue };
        let Some(title) = parent.file_name().and_then(|n| n.to_str()) else {
            continue;
        };

        let chapter = if let Some(c) = extract_prefixed_chapter(&chapter_prefixed, stem) {
            Some(c)
        } else if !stem.to_lowercase().contains("vol") {
            // bare-numbered chapters have no boundary ambiguity to check:
            // the pattern already anchors to an immediately-following
            // "(YYYY)" year, so any match is a confident one.
            bare_numbered.captures(stem).map(|c| c[1].to_string())
        } else {
            None
        };

        let Some(chapter) = chapter else {
            skipped += 1;
            continue;
        };
        let volume = volume_re.captures(stem).map(|c| c[1].to_string());

        let new_filename = cbz::cbz_filename(title, volume.as_deref(), Some(&chapter));
        let new_path = parent.join(&new_filename);

        if new_path == path {
            continue;
        }
        if new_path.exists() {
            eprintln!(
                "collision, skipping: {} -> {} (destination already exists)",
                path.display(),
                new_path.display()
            );
            collisions += 1;
            continue;
        }

        println!("{} -> {}", path.display(), new_path.display());
        if apply {
            std::fs::rename(&path, &new_path)?;
        }
        renamed += 1;
    }

    println!(
        "\n{}: {renamed} renamed, {skipped} left as-is (no confident chapter number), {collisions} collisions",
        if apply { "applied" } else { "dry run (pass --apply to execute)" }
    );

    Ok(())
}

/// Finds the `c<chapter>` token and requires it be immediately followed by
/// whitespace, `(`, or end-of-string — rejects things like `c57-58` (a
/// chapter range, not a single chapter) that the regex's lack of look-around
/// can't reject on its own.
fn extract_prefixed_chapter(re: &Regex, stem: &str) -> Option<String> {
    let caps = re.captures(stem)?;
    let m = caps.get(1)?;
    match stem[m.end()..].chars().next() {
        None => Some(m.as_str().to_string()),
        Some(c) if c.is_whitespace() || c == '(' => Some(m.as_str().to_string()),
        _ => None,
    }
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) -> anyhow::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            walk(&path, out)?;
        } else if path.extension().and_then(|e| e.to_str()) == Some("cbz") {
            out.push(path);
        }
    }
    Ok(())
}

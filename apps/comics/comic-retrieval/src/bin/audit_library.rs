//! Library-wide audit, run manually rather than on a schedule.
//!
//! Walks every series folder under NAS_COMICS_PATH and reports two things:
//!
//!   1. Single-file series — a folder with exactly one comic file in it.
//!      This can't be told apart from "a real series that's only released
//!      one chapter so far" using the filesystem alone, so these are only
//!      ever reported, never touched — they're for a human to cross-check
//!      against MangaDex (is this tagged as a genuine one-shot, or just
//!      early in its run?) and decide whether the catalog entry is right.
//!
//!   2. Volumes/chapters out of order — a folder that has both whole-volume
//!      files (from `backfill`, or from the older per-volume nyaa releases
//!      `fix_scene_names` deliberately left alone) and individual chapter
//!      files, where a plain filename sort doesn't put every volume before
//!      every chapter. `backfill`'s own `bulk_volume_filename` already sorts
//!      correctly by construction (leading uppercase `V`); the case this
//!      catches is older per-volume releases using a lowercase `v`, which
//!      sorts *after* the lowercase `c` chapter files use — e.g. "Black
//!      Butler v27 (2019) (Digital) (Shizu).cbz" next to "Black Butler
//!      c0300.cbz". Fixed by uppercasing that leading `v` — a one-character,
//!      fully information-preserving change. Anything that doesn't match
//!      this specific, safe pattern (e.g. a volume number embedded before
//!      the title, like "v01_Pandora Hearts [digital]...cbz") is reported
//!      but left alone, same principle as `fix_scene_names`.
//!
//! Defaults to a dry run. Pass `--apply` to actually rename files.

use regex::Regex;
use std::path::PathBuf;

struct Classified {
    path: PathBuf,
    kind: Kind,
}

#[derive(PartialEq, Eq, Debug)]
enum Kind {
    Chapter,
    /// Volume-shaped, already sorts correctly (bulk_volume_filename's
    /// leading `V`, or a legacy "Vol. NN" spelling — already uppercase).
    VolumeOk,
    /// Volume-shaped, sorts wrong today, safely fixable by uppercasing the
    /// single leading `v`.
    VolumeFixable,
    /// Volume-shaped by a looser heuristic, but not in the specific
    /// "{title} v<digit>" shape the fix knows how to correct safely.
    VolumeUnfixable,
    Unclassified,
}

fn main() -> anyhow::Result<()> {
    let apply = std::env::args().any(|a| a == "--apply");
    let root = std::env::var("NAS_COMICS_PATH").unwrap_or_else(|_| "/books/manga".to_string());
    println!("auditing {root}");

    // pad_chapter_num (library::cbz) always produces a leading 4-digit block
    // right after " c" — for anything it can't cleanly parse as a plain
    // number (split-chapter letters like "37a", multi-segment decimals like
    // "12.1.5", legacy chapter ranges like "57-58") it still pads that
    // leading digit run to 4 digits and leaves the rest verbatim. So rather
    // than trying to enumerate every shape that can follow, just anchor on
    // what's actually guaranteed: " c" + 4 digits, then anything, then
    // ".cbz". A narrower pattern here previously misclassified exactly
    // those exotic-but-legitimate chapter shapes as unrecognized volumes.
    let chapter_re = Regex::new(r" c[0-9]{4}.*\.cbz$")?;
    let bulk_ok_re = Regex::new(r" V[0-9]{4}(\.[0-9]{3})?-[0-9]{4}(\.[0-9]{3})? \(v[0-9.]+\)\.cbz$")?;
    // No trailing `\b`: underscore counts as a word character, so e.g.
    // "v01_Title...cbz" has no boundary between "1" and "_" and would
    // silently fail to match if one were required here.
    let vol_word_re = Regex::new(r"(?i)\bv(?:ol(?:ume)?)?\.?\s*0*(\d+)")?;

    let mut single_file_series = Vec::new();
    let mut ordering_violations = Vec::new();
    let mut renamed = 0usize;
    let mut series_scanned = 0usize;

    for entry in std::fs::read_dir(&root)? {
        let entry = entry?;
        let dir_path = entry.path();
        if !dir_path.is_dir() {
            continue;
        }
        let Some(title) = dir_path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        series_scanned += 1;

        let mut files: Vec<PathBuf> = std::fs::read_dir(&dir_path)?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| matches!(p.extension().and_then(|e| e.to_str()), Some("cbz") | Some("zip")))
            .collect();
        files.sort();

        if files.len() == 1 {
            single_file_series.push((title.to_string(), files[0].clone()));
            continue;
        }
        if files.is_empty() {
            continue;
        }

        let classified: Vec<Classified> = files
            .into_iter()
            .map(|path| {
                let filename = path.file_name().and_then(|f| f.to_str()).unwrap_or("");
                let kind = if bulk_ok_re.is_match(filename) {
                    Kind::VolumeOk
                } else if chapter_re.is_match(filename) {
                    Kind::Chapter
                } else if let Some(rest) = filename.strip_prefix(&format!("{title} v")) {
                    if rest.chars().next().is_some_and(|c| c.is_ascii_digit()) {
                        Kind::VolumeFixable
                    } else {
                        Kind::Unclassified
                    }
                } else if vol_word_re.is_match(filename) {
                    Kind::VolumeUnfixable
                } else {
                    Kind::Unclassified
                };
                Classified { path, kind }
            })
            .collect();

        let has_chapter = classified.iter().any(|c| c.kind == Kind::Chapter);
        let has_volume = classified.iter().any(|c| {
            matches!(c.kind, Kind::VolumeOk | Kind::VolumeFixable | Kind::VolumeUnfixable)
        });
        if !has_chapter || !has_volume {
            continue;
        }

        // sorted-by-filename order must have every volume before every chapter
        let mut sorted = classified;
        sorted.sort_by(|a, b| a.path.cmp(&b.path));
        let last_volume_idx = sorted
            .iter()
            .rposition(|c| matches!(c.kind, Kind::VolumeOk | Kind::VolumeFixable | Kind::VolumeUnfixable));
        let first_chapter_idx = sorted.iter().position(|c| c.kind == Kind::Chapter);
        let (Some(last_vol), Some(first_ch)) = (last_volume_idx, first_chapter_idx) else {
            continue;
        };
        if last_vol < first_ch {
            continue; // already correctly ordered
        }

        let fixable: Vec<&Classified> = sorted
            .iter()
            .filter(|c| c.kind == Kind::VolumeFixable)
            .collect();
        let unfixable: Vec<&Classified> = sorted
            .iter()
            .filter(|c| c.kind == Kind::VolumeUnfixable)
            .collect();

        for c in &fixable {
            let filename = c.path.file_name().and_then(|f| f.to_str()).unwrap_or("");
            let fixed_filename = uppercase_leading_v(filename, title);
            let new_path = c.path.with_file_name(&fixed_filename);
            println!("{} -> {}", c.path.display(), new_path.display());
            if apply {
                if new_path.exists() {
                    eprintln!("  skip: destination already exists");
                } else {
                    std::fs::rename(&c.path, &new_path)?;
                    renamed += 1;
                }
            } else {
                renamed += 1;
            }
        }

        ordering_violations.push((
            title.to_string(),
            fixable.len(),
            unfixable.iter().map(|c| c.path.display().to_string()).collect::<Vec<_>>(),
        ));
    }

    println!("\n=== single-file series ({}) ===", single_file_series.len());
    println!("(can't tell a true one-shot from an ongoing series with 1 chapter so far from files alone — for manual review)");
    for (title, path) in &single_file_series {
        println!("  {title}: {}", path.file_name().and_then(|f| f.to_str()).unwrap_or(""));
    }

    println!("\n=== volume/chapter ordering violations ({}) ===", ordering_violations.len());
    for (title, fixed, unfixable) in &ordering_violations {
        println!("  {title}: {fixed} volume file(s) {}", if apply { "fixed" } else { "would fix" });
        for f in unfixable {
            println!("    unresolved (needs manual review): {f}");
        }
    }

    println!(
        "\n{}: {series_scanned} series scanned, {} single-file, {} ordering violations, {renamed} volume file(s) {}",
        if apply { "applied" } else { "dry run (pass --apply to execute)" },
        single_file_series.len(),
        ordering_violations.len(),
        if apply { "renamed" } else { "would rename" }
    );

    Ok(())
}

/// Uppercases the leading `v` in "{title} v<digits>..." — the one character
/// that determines whether this sorts before or after `c`-prefixed chapter
/// files of the same series. Everything else in the filename is untouched.
fn uppercase_leading_v(filename: &str, title: &str) -> String {
    let prefix_len = format!("{title} v").len();
    let mut chars: Vec<char> = filename.chars().collect();
    if prefix_len > 0 && prefix_len - 1 < chars.len() {
        chars[prefix_len - 1] = 'V';
    }
    chars.into_iter().collect()
}

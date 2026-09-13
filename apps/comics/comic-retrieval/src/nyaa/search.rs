use anyhow::{Context, Result};
use tracing::{debug, info};

const NYAA_RSS: &str = "https://nyaa.si/?page=rss&f=0&c=3_1";

#[derive(Debug, Clone)]
pub struct NyaaResult {
    pub title: String,
    /// Direct URL to the .torrent file (e.g. https://nyaa.si/download/123456.torrent)
    pub torrent_url: String,
    pub seeders: u32,
    pub size_bytes: Option<u64>,
}

/// Search nyaa.si English-translated manga category for the given title.
/// Returns results sorted by seeder count descending.
pub async fn search(client: &reqwest::Client, manga_title: &str) -> Result<Vec<NyaaResult>> {
    let query = urlencoding::encode(manga_title).into_owned();
    let url = format!("{NYAA_RSS}&q={query}");

    debug!("nyaa search: {url}");

    let xml = client
        .get(&url)
        .send()
        .await
        .context("nyaa request")?
        .text()
        .await
        .context("nyaa response body")?;

    let channel = rss::Channel::read_from(xml.as_bytes())
        .context("parse nyaa RSS")?;

    let mut results: Vec<NyaaResult> = channel
        .items()
        .iter()
        .filter_map(|item| parse_item(item))
        .collect();

    // sort best seeders first
    results.sort_by(|a, b| b.seeders.cmp(&a.seeders));

    info!(
        "nyaa '{}': {} results, best has {} seeders",
        manga_title,
        results.len(),
        results.first().map(|r| r.seeders).unwrap_or(0)
    );

    Ok(results)
}

/// Pick the best result for a manga title: must contain the key words of the
/// title (case-insensitive), have the most seeders, and not look like a
/// whole-series/whole-volume-range batch. This fallback exists to plug a
/// single missing chapter — a popular "vol 1-28 complete" torrent will often
/// outrank a single-chapter release on seeders alone, and grabbing it would
/// dump dozens of arbitrarily-named files into the manga's folder instead.
pub fn best_match<'a>(results: &'a [NyaaResult], manga_title: &str) -> Option<&'a NyaaResult> {
    let keywords = title_keywords(manga_title);

    results.iter().find(|r| {
        let lower = r.title.to_lowercase();
        keywords.iter().all(|kw| lower.contains(kw.as_str())) && !looks_like_bulk_release(&lower)
    })
}

/// Pick the best *bulk* result for a manga title: must contain the key words
/// of the title and, unlike `best_match`, must look like a whole-series or
/// whole-volume-range batch. For backfilling a run of older volumes MangaDex
/// doesn't host at all (see `backfill`), a single-chapter torrent isn't the
/// right shape of result — a "v01-20 complete" release is exactly what's
/// wanted here, the opposite of what `best_match` filters out.
pub fn best_bulk_match<'a>(results: &'a [NyaaResult], manga_title: &str) -> Option<&'a NyaaResult> {
    let keywords = title_keywords(manga_title);

    results.iter().find(|r| {
        let lower = r.title.to_lowercase();
        keywords.iter().all(|kw| lower.contains(kw.as_str())) && looks_like_bulk_release(&lower)
    })
}

/// Matches each of `volumes` to the best-seeded nyaa result whose title
/// contains the manga's title keywords and that specific volume number.
///
/// In practice nyaa overwhelmingly releases manga one torrent per volume —
/// "Series v33 (2024) (Digital) (Group)" — rather than as a single "v01-20
/// complete" bundle (`best_bulk_match` exists for the rare case where a real
/// bundle does exist, but it essentially never does). So backfilling a run
/// of missing volumes means finding each volume's own torrent individually
/// rather than looking for one torrent that covers them all. Volumes with no
/// matching result are simply omitted — the caller decides what, if
/// anything, to do about a partially-covered gap.
pub fn match_volumes<'a>(
    results: &'a [NyaaResult],
    manga_title: &str,
    volumes: &[String],
) -> Vec<(String, &'a NyaaResult)> {
    let keywords = title_keywords(manga_title);
    let vol_re = regex::Regex::new(r"(?i)\bv(?:ol(?:ume)?)?\.?\s*0*(\d+)\b").expect("valid regex");

    let mut matches = Vec::new();
    for vol in volumes {
        let found = results.iter().find(|r| {
            if !keywords
                .iter()
                .all(|kw| r.title.to_lowercase().contains(kw.as_str()))
            {
                return false;
            }
            vol_re
                .captures(&r.title)
                .map(|c| &c[1] == vol.as_str())
                .unwrap_or(false)
        });
        if let Some(r) = found {
            matches.push((vol.clone(), r));
        }
    }
    matches
}

/// True if a (lowercased) torrent title looks like a whole-series or
/// whole-volume-range batch rather than something that could plausibly be a
/// single chapter: explicit "complete"/"batch"/"omnibus" wording, or a
/// volume/chapter range like "v01-28" / "vol. 1-10" / "volumes 1-74".
pub(crate) fn looks_like_bulk_release(lower_title: &str) -> bool {
    const BULK_WORDS: &[&str] = &["complete", "batch", "omnibus", "box set", "boxset"];
    if BULK_WORDS.iter().any(|w| lower_title.contains(w)) {
        return true;
    }
    let range_re = regex::Regex::new(r"v(?:ol(?:ume)?s?)?\.?\s*\d+\s*[-–]\s*\d+")
        .expect("valid regex");
    range_re.is_match(lower_title)
}

fn parse_item(item: &rss::Item) -> Option<NyaaResult> {
    let title = item.title()?.to_string();

    // torrent URL comes from the <enclosure> element
    let torrent_url = item.enclosure()?.url().to_string();

    let seeders = item
        .extensions()
        .get("nyaa")
        .and_then(|ns| ns.get("seeders"))
        .and_then(|v| v.first())
        .and_then(|ext| ext.value())
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(0);

    let size_bytes = item
        .extensions()
        .get("nyaa")
        .and_then(|ns| ns.get("size"))
        .and_then(|v| v.first())
        .and_then(|ext| ext.value())
        .and_then(|v| parse_size_str(v));

    Some(NyaaResult { title, torrent_url, seeders, size_bytes })
}

/// Parse nyaa size strings like "500 MiB", "1.2 GiB", "300 KiB" into bytes.
fn parse_size_str(s: &str) -> Option<u64> {
    let parts: Vec<&str> = s.split_whitespace().collect();
    if parts.len() != 2 {
        return None;
    }
    let n: f64 = parts[0].parse().ok()?;
    let bytes = match parts[1] {
        "KiB" => n * 1024.0,
        "MiB" => n * 1024.0 * 1024.0,
        "GiB" => n * 1024.0 * 1024.0 * 1024.0,
        "TiB" => n * 1024.0 * 1024.0 * 1024.0 * 1024.0,
        _ => return None,
    };
    Some(bytes as u64)
}

/// Extract meaningful keywords from a title (strips short words).
fn title_keywords(title: &str) -> Vec<String> {
    title
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() >= 3)
        .map(|w| w.to_lowercase())
        .collect()
}

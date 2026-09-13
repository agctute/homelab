use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

use super::client::MangaDexClient;

/// A volume MangaDex knows about but doesn't host any chapter of directly —
/// a candidate for bulk backfill via torrent instead of per-chapter download.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VolumeGap {
    pub volume: String,
    pub first_chapter: String,
    pub last_chapter: String,
    pub chapter_count: usize,
}

/// Fetches MangaDex's volume/chapter aggregate for a manga and returns the
/// volumes where none of the known chapter numbers appear in
/// `known_chapter_nums` (the chapters we've already queued or downloaded).
/// A volume with even one chapter present is left to the normal per-chapter
/// pipeline — bulk backfill is for volumes MangaDex doesn't host at all, not
/// for patching a single stubborn gap.
pub async fn fetch_missing_volumes(
    client: &MangaDexClient,
    manga_id: &str,
    known_chapter_nums: &HashSet<String>,
) -> Result<Vec<VolumeGap>> {
    let url = client.api_url(&format!(
        "/manga/{manga_id}/aggregate?translatedLanguage%5B%5D=en"
    ));
    let resp: serde_json::Value = client.get_json(&url).await?;
    Ok(diff_aggregate(&resp, known_chapter_nums))
}

fn diff_aggregate(resp: &serde_json::Value, known_chapter_nums: &HashSet<String>) -> Vec<VolumeGap> {
    let Some(volumes) = resp["volumes"].as_object() else {
        return Vec::new();
    };

    let mut gaps = Vec::new();
    for (vol_key, vol_val) in volumes {
        if vol_key == "none" {
            continue;
        }
        let Some(chapters_obj) = vol_val["chapters"].as_object() else {
            continue;
        };
        if chapters_obj.is_empty() {
            continue;
        }
        if chapters_obj
            .keys()
            .any(|c| known_chapter_nums.contains(c.as_str()))
        {
            continue;
        }

        let mut sorted: Vec<&String> = chapters_obj.keys().collect();
        sorted.sort_by(|a, b| {
            let na = a.parse::<f64>().unwrap_or(f64::MAX);
            let nb = b.parse::<f64>().unwrap_or(f64::MAX);
            na.partial_cmp(&nb).unwrap_or(std::cmp::Ordering::Equal)
        });
        let (Some(first), Some(last)) = (sorted.first(), sorted.last()) else {
            continue;
        };

        gaps.push(VolumeGap {
            volume: vol_key.clone(),
            first_chapter: first.to_string(),
            last_chapter: last.to_string(),
            chapter_count: chapters_obj.len(),
        });
    }

    gaps.sort_by(|a, b| {
        let na = a.volume.parse::<f64>().unwrap_or(f64::MAX);
        let nb = b.volume.parse::<f64>().unwrap_or(f64::MAX);
        na.partial_cmp(&nb).unwrap_or(std::cmp::Ordering::Equal)
    });
    gaps
}

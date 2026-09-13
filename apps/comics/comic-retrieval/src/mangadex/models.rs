use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Manga {
    pub id: String,
    pub attributes: MangaAttributes,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MangaAttributes {
    pub title: std::collections::HashMap<String, String>,
    #[serde(default)]
    pub alt_titles: Vec<std::collections::HashMap<String, String>>,
    pub original_language: String,
    pub status: String,
    pub last_chapter: Option<String>,
}

/// English and (romanized-preferred) Japanese names resolved independently,
/// plus the combined "{English} ({Japanese})" form used for the catalog
/// title / folder name when both are available and differ.
pub struct DualTitle {
    pub english: Option<String>,
    pub japanese: Option<String>,
    pub combined: String,
}

impl Manga {
    pub fn english_title(&self) -> String {
        self.attributes
            .title
            .get("en")
            .or_else(|| self.attributes.title.get("ja-ro"))
            .or_else(|| self.attributes.title.values().next())
            .cloned()
            .unwrap_or_else(|| self.id.clone())
    }

    /// Looks in the primary `title` map first, then each `altTitles` entry,
    /// for the first of `keys` present — `title` before `altTitles` since
    /// it's the "real" title MangaDex assigns; alt titles are supplementary.
    fn find_title(&self, keys: &[&str]) -> Option<String> {
        for key in keys {
            if let Some(v) = self.attributes.title.get(*key) {
                return Some(v.clone());
            }
        }
        for alt in &self.attributes.alt_titles {
            for key in keys {
                if let Some(v) = alt.get(*key) {
                    return Some(v.clone());
                }
            }
        }
        None
    }

    pub fn dual_title(&self) -> DualTitle {
        let english = self.find_title(&["en"]);
        // Prefer romanized Japanese (Latin script, filesystem-friendly) —
        // only fall back to native script if no romanization is recorded.
        let japanese = self.find_title(&["ja-ro"]).or_else(|| self.find_title(&["ja"]));

        let combined = match (&english, &japanese) {
            (Some(e), Some(j)) if e != j => format!("{e} ({j})"),
            (Some(e), _) => e.clone(),
            (None, Some(j)) => j.clone(),
            (None, None) => self.id.clone(),
        };

        // Most filesystems cap filenames at 255 bytes, and this string
        // becomes not just the folder name but the prefix of every chapter
        // filename inside it (which appends another ~20-30 bytes for
        // " cNNNN (vNN).cbz") — some titles' combined English+Japanese
        // names genuinely exceed that. Prefer dropping the Japanese
        // parenthetical over a garbled mid-word truncation; only hard-cut
        // as a last resort if English alone still doesn't fit.
        const MAX_TITLE_BYTES: usize = 200;
        let combined = if combined.len() > MAX_TITLE_BYTES {
            match &english {
                Some(e) if e.len() <= MAX_TITLE_BYTES => e.clone(),
                Some(e) => truncate_to_byte_limit(e, MAX_TITLE_BYTES),
                None => truncate_to_byte_limit(&combined, MAX_TITLE_BYTES),
            }
        } else {
            combined
        };

        DualTitle { english, japanese, combined }
    }
}

/// Truncates `s` to at most `max_bytes` bytes, backing off to the nearest
/// UTF-8 character boundary and trimming trailing whitespace so a
/// multi-byte character (e.g. Japanese script) never gets split mid-way.
fn truncate_to_byte_limit(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].trim_end().to_string()
}

#[derive(Debug, Deserialize)]
pub struct Chapter {
    pub id: String,
    pub attributes: ChapterAttributes,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChapterAttributes {
    pub chapter: Option<String>,
    pub volume: Option<String>,
    pub translated_language: String,
    pub pages: u32,
    pub external_url: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AtHomeResponse {
    pub base_url: String,
    pub chapter: AtHomeChapter,
}

#[derive(Debug, Deserialize)]
pub struct AtHomeChapter {
    pub hash: String,
    pub data: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct AuthResponse {
    pub access_token: String,
    pub expires_in: u64,
    pub refresh_token: Option<String>,
}

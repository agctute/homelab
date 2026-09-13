pub mod google_books;

/// Enrichment data pulled from a book-database lookup, fed into the
/// classifier alongside the raw torrent name so it has real author/
/// publisher/genre signal instead of guessing purely from a scene-release
/// filename (e.g. inferring comics-vs-manga country of origin).
#[derive(Debug, Clone, Default)]
pub struct BookMetadata {
    pub title: Option<String>,
    pub authors: Vec<String>,
    pub publisher: Option<String>,
    pub description: Option<String>,
    pub categories: Vec<String>,
    pub language: Option<String>,
}

/// Strips common scene-release noise (bracketed/parenthetical tags, which
/// usually carry group names, resolution, year, volume ranges, etc.) so the
/// remainder is a reasonable search query for a book-metadata API.
/// Best-effort — not a full release-name parser.
pub fn guess_search_title(torrent_name: &str) -> String {
    let cut = torrent_name.find(['(', '[']).unwrap_or(torrent_name.len());
    torrent_name[..cut].trim().to_string()
}

pub mod openai;

use anyhow::Result;
use async_trait::async_trait;

/// What a registered app tells the classifier about itself, so the LLM
/// prompt can be built generically from whatever apps are registered.
pub struct AppInfo {
    pub id: &'static str,
    pub description: &'static str,
}

pub struct ClassificationInput<'a> {
    pub torrent_name: &'a str,
    pub category: Option<&'a str>,
    /// Actual file extension(s) found in the torrent's downloaded content
    /// (see `fsutil::list_extensions`) — a hard signal of format (e.g.
    /// `.m4b` = audio) versus inferring it from the torrent's name, which
    /// often carries no extension at all.
    pub file_extensions: &'a [String],
    /// Best-effort enrichment from a book-database lookup (see
    /// `crate::metadata`). `None` when the lookup found nothing or wasn't
    /// attempted — classification must still work from `torrent_name` alone.
    pub book_metadata: Option<&'a crate::metadata::BookMetadata>,
}

#[derive(Debug, Clone)]
pub struct Classification {
    /// One of the candidate app ids, or "unknown" if nothing matched.
    pub app_id: String,
    /// Series name (comics/manga) or book title (novels/textbooks), cleaned
    /// of scene/release-group tags. Used as the destination directory name.
    pub title: Option<String>,
    /// Best-effort detected language (e.g. "English", "Japanese"), used to
    /// enforce the English-only rule before handing off to an app.
    pub language: Option<String>,
    pub reasoning: Option<String>,
}

/// Decides which registered app a completed torrent belongs to, and pulls
/// out the handful of fields every app needs (title, language) so that work
/// isn't duplicated per app handler.
#[async_trait]
pub trait Classifier: Send + Sync {
    async fn classify(
        &self,
        input: ClassificationInput<'_>,
        candidates: &[AppInfo],
    ) -> Result<Classification>;
}

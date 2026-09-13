use super::{AppHandler, CompletedTorrent};
use crate::classifier::Classification;
use crate::config::Config;
use crate::fsutil;
use anyhow::{Context, Result};
use async_trait::async_trait;
use std::path::Path;
use tracing::warn;

const AUDIO_EXTENSIONS: &[&str] = &["m4b", "mp3", "m4a", "flac", "aac", "ogg", "wav"];

/// Audiobooks: unlike novels/textbooks, there's no single target format to
/// convert into — audio formats vary (m4b, mp3, ...) and transcoding would
/// degrade quality for no benefit, so files are copied as-is rather than
/// converted. A single audiobook may be one file (.m4b) or a directory of
/// per-chapter files (.mp3), so — unlike prose's "largest file" heuristic —
/// whole directories are copied intact rather than picking one file out.
pub struct AudiobookLibraryApp;

pub fn app() -> AudiobookLibraryApp {
    AudiobookLibraryApp
}

#[async_trait]
impl AppHandler for AudiobookLibraryApp {
    fn id(&self) -> &'static str {
        "audiobooks"
    }

    fn description(&self) -> &'static str {
        "Audiobooks — narrated/spoken-word AUDIO recordings of books, not text. Strong \
         signals this is the right app: the filename/extension is an audio format (.m4b, \
         .mp3, .m4a, .flac, .aac, .ogg, .wav) rather than a text/ebook format (.epub, \
         .mobi, .pdf, .azw3); or the name mentions \"audiobook\", \"unabridged\", \
         \"narrated by\", or a narrator's name. A book with the exact same title/author \
         released as a text file (epub/mobi/pdf) belongs to `novels` or `textbooks` \
         instead — judge by the FILE FORMAT the release is actually in, not just the \
         book's title."
    }

    async fn handle(
        &self,
        torrent: &CompletedTorrent,
        classification: &Classification,
        config: &Config,
    ) -> Result<()> {
        let title = fsutil::sanitize(classification.title.as_deref().unwrap_or(&torrent.name));
        let item_dir = Path::new(config.audiobooks_path.as_str()).join(&title);
        fsutil::ensure_dir(&item_dir).await?;

        let content = torrent.content_path.as_path();

        if content.is_dir() {
            let mut entries = tokio::fs::read_dir(content)
                .await
                .context("read torrent directory")?;
            while let Some(entry) = entries.next_entry().await? {
                let path = entry.path();
                if path.is_dir() {
                    warn!(item = %path.display(), "nested directory in audiobook torrent, skipping");
                    continue;
                }
                copy_if_audio(&path, &item_dir).await?;
            }
        } else {
            copy_if_audio(content, &item_dir).await?;
        }

        Ok(())
    }
}

async fn copy_if_audio(src: &Path, item_dir: &Path) -> Result<()> {
    let is_audio = src
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .is_some_and(|e| AUDIO_EXTENSIONS.contains(&e.as_str()));

    if !is_audio {
        warn!(item = %src.display(), "non-audio file in audiobook torrent, leaving in place");
        return Ok(());
    }

    let filename = src.file_name().context("audio file has no filename")?;
    fsutil::copy_file(src, &item_dir.join(filename)).await
}

use super::{AppHandler, CompletedTorrent};
use crate::classifier::Classification;
use crate::config::Config;
use crate::format::convert::{ExternalConverter, TargetFormat};
use crate::format::{self, SourceFormat};
use crate::fsutil;
use anyhow::{Context, Result};
use async_trait::async_trait;
use std::path::{Path, PathBuf};

/// Shared behavior for novels and textbooks: each item is a single file
/// (EPUB for novels, PDF for textbooks) living in its own directory — unlike
/// comics/manga, items are never grouped together.
pub struct ProseLibraryApp {
    id: &'static str,
    description: &'static str,
    dest_root: fn(&Config) -> &str,
    target: TargetFormat,
}

impl ProseLibraryApp {
    pub fn new(
        id: &'static str,
        description: &'static str,
        dest_root: fn(&Config) -> &str,
        target: TargetFormat,
    ) -> Self {
        Self {
            id,
            description,
            dest_root,
            target,
        }
    }
}

#[async_trait]
impl AppHandler for ProseLibraryApp {
    fn id(&self) -> &'static str {
        self.id
    }

    fn description(&self) -> &'static str {
        self.description
    }

    async fn handle(
        &self,
        torrent: &CompletedTorrent,
        classification: &Classification,
        config: &Config,
    ) -> Result<()> {
        let title = fsutil::sanitize(classification.title.as_deref().unwrap_or(&torrent.name));
        let item_dir = Path::new((self.dest_root)(config)).join(&title);
        fsutil::ensure_dir(&item_dir).await?;

        let converter = ExternalConverter::new(
            config.zip_bin.clone(),
            config.unrar_bin.clone(),
            config.ebook_convert_bin.clone(),
        );
        let content = torrent.content_path.as_path();
        let dest = item_dir.join(format!("{title}.{}", self.target.extension()));

        let source = if content.is_dir() {
            find_primary_file(content).await?
        } else {
            content.to_path_buf()
        };

        if matches_target(format::detect(&source), self.target) {
            fsutil::copy_file(&source, &dest).await?;
        } else {
            converter.convert(&source, &dest, self.target).await?;
        }

        Ok(())
    }
}

fn matches_target(fmt: SourceFormat, target: TargetFormat) -> bool {
    matches!(
        (fmt, target),
        (SourceFormat::Epub, TargetFormat::Epub) | (SourceFormat::Pdf, TargetFormat::Pdf)
    )
}

/// A torrent for a single novel/textbook sometimes lands as a folder (cover
/// art, an .nfo, the actual book). Pick the largest file as the book itself
/// — good enough for a scaffold; revisit if this misfires in practice.
async fn find_primary_file(dir: &Path) -> Result<PathBuf> {
    let mut entries = tokio::fs::read_dir(dir).await.context("read torrent directory")?;
    let mut best: Option<(PathBuf, u64)> = None;
    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if path.is_dir() {
            continue;
        }
        let size = entry.metadata().await.map(|m| m.len()).unwrap_or(0);
        if best.as_ref().map(|(_, s)| size > *s).unwrap_or(true) {
            best = Some((path, size));
        }
    }
    best.map(|(p, _)| p).context("no files found in torrent directory")
}

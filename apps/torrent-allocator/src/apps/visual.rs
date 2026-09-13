use super::{AppHandler, CompletedTorrent};
use crate::classifier::Classification;
use crate::config::Config;
use crate::format::convert::{ExternalConverter, TargetFormat};
use crate::format::{self, SourceFormat};
use crate::fsutil;
use anyhow::{Context, Result};
use async_trait::async_trait;
use std::path::Path;
use tracing::warn;

/// Shared behavior for comics and manga: both store every edition/volume/
/// chapter as its own .cbz, with every .cbz belonging to a series living
/// together in one directory. The only difference between the two apps is
/// the id, the LLM-facing description (reading direction is the
/// discriminator), and the destination root.
pub struct VisualLibraryApp {
    id: &'static str,
    description: &'static str,
    dest_root: fn(&Config) -> &str,
}

impl VisualLibraryApp {
    pub fn new(id: &'static str, description: &'static str, dest_root: fn(&Config) -> &str) -> Self {
        Self {
            id,
            description,
            dest_root,
        }
    }

    async fn place_one(&self, item: &Path, series_dir: &Path, converter: &ExternalConverter) -> Result<()> {
        match format::detect(item) {
            SourceFormat::Cbz | SourceFormat::Zip => {
                // A cbz IS a zip archive — a mislabeled .zip just needs a
                // renamed copy, no real conversion required.
                let raw = item
                    .file_stem()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let dest = series_dir.join(format!("{}.cbz", fsutil::sanitize(&raw)));
                fsutil::copy_file(item, &dest).await?;
            }
            SourceFormat::ImageFolder => {
                let raw = item
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let dest = series_dir.join(format!("{}.cbz", fsutil::sanitize(&raw)));
                converter.convert(item, &dest, TargetFormat::Cbz).await?;
            }
            SourceFormat::Cbr | SourceFormat::Rar => {
                let raw = item
                    .file_stem()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let dest = series_dir.join(format!("{}.cbz", fsutil::sanitize(&raw)));
                converter.convert(item, &dest, TargetFormat::Cbz).await?;
            }
            other => {
                warn!(
                    item = %item.display(),
                    format = ?other,
                    "unrecognized comics/manga item, leaving in place"
                );
            }
        }
        Ok(())
    }
}

#[async_trait]
impl AppHandler for VisualLibraryApp {
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
        let series = fsutil::sanitize(classification.title.as_deref().unwrap_or(&torrent.name));
        let series_dir = Path::new((self.dest_root)(config)).join(&series);
        fsutil::ensure_dir(&series_dir).await?;

        let converter = ExternalConverter::new(
            config.zip_bin.clone(),
            config.unrar_bin.clone(),
            config.ebook_convert_bin.clone(),
        );
        let content = torrent.content_path.as_path();

        if content.is_dir() {
            match format::detect(content) {
                SourceFormat::ImageFolder => {
                    // The whole torrent is a single volume/chapter of loose images.
                    let name = fsutil::sanitize(&torrent.name);
                    let dest = series_dir.join(format!("{name}.cbz"));
                    converter.convert(content, &dest, TargetFormat::Cbz).await?;
                }
                _ => {
                    // A folder containing multiple volumes/chapters — handle each
                    // immediate child independently.
                    let mut entries = tokio::fs::read_dir(content)
                        .await
                        .context("read torrent directory")?;
                    while let Some(entry) = entries.next_entry().await? {
                        self.place_one(&entry.path(), &series_dir, &converter).await?;
                    }
                }
            }
        } else {
            self.place_one(content, &series_dir, &converter).await?;
        }

        Ok(())
    }
}

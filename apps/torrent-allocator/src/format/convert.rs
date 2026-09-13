use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use tokio::process::Command;
use tracing::info;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetFormat {
    Cbz,
    Epub,
    Pdf,
}

impl TargetFormat {
    pub fn extension(self) -> &'static str {
        match self {
            TargetFormat::Cbz => "cbz",
            TargetFormat::Epub => "epub",
            TargetFormat::Pdf => "pdf",
        }
    }
}

/// Converts content into a target format using well-known, real tools
/// (rather than the placeholder `comicony`/`bookvert` names used earlier):
/// - CBZ: a CBZ is just a zip archive, so a folder of loose images is
///   `zip`-ed directly; a CBR/RAR archive is extracted with `unrar` first,
///   then the result is zipped.
/// - EPUB/PDF: Calibre's `ebook-convert` handles arbitrary ebook formats
///   (mobi, azw3, epub, pdf, txt, ...) via a single `ebook-convert <in>
///   <out>` invocation — the output file's extension tells it the target.
pub struct ExternalConverter {
    pub zip_bin: String,
    pub unrar_bin: String,
    pub ebook_convert_bin: String,
}

impl ExternalConverter {
    pub fn new(zip_bin: String, unrar_bin: String, ebook_convert_bin: String) -> Self {
        Self {
            zip_bin,
            unrar_bin,
            ebook_convert_bin,
        }
    }

    pub async fn convert(&self, input: &Path, output: &Path, target: TargetFormat) -> Result<()> {
        if let Some(parent) = output.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .context("create output directory")?;
        }

        match target {
            TargetFormat::Cbz => self.convert_to_cbz(input, output).await,
            TargetFormat::Epub | TargetFormat::Pdf => {
                self.run(
                    &self.ebook_convert_bin,
                    &[
                        input.to_string_lossy().into_owned(),
                        output.to_string_lossy().into_owned(),
                    ],
                )
                .await
            }
        }
    }

    async fn convert_to_cbz(&self, input: &Path, output: &Path) -> Result<()> {
        if input.is_dir() {
            return self.zip_dir(input, output).await;
        }

        // A single archive file (.cbr/.rar) — extract to a scratch dir next
        // to the destination, zip that into the .cbz, then clean up.
        let parent = output.parent().context("output has no parent directory")?;
        let stem = input
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let tmp: PathBuf = parent.join(format!(".extract-{stem}"));
        tokio::fs::create_dir_all(&tmp)
            .await
            .context("create extraction temp dir")?;

        let outcome = async {
            self.run(
                &self.unrar_bin,
                &[
                    "x".to_string(),
                    "-o+".to_string(),
                    input.to_string_lossy().into_owned(),
                    format!("{}/", tmp.display()),
                ],
            )
            .await?;
            self.zip_dir(&tmp, output).await
        }
        .await;

        let _ = tokio::fs::remove_dir_all(&tmp).await;
        outcome
    }

    async fn zip_dir(&self, dir: &Path, output: &Path) -> Result<()> {
        // zip updates an existing archive in place rather than replacing it
        // — remove any stale output first so this is a clean overwrite.
        if tokio::fs::metadata(output).await.is_ok() {
            let _ = tokio::fs::remove_file(output).await;
        }

        self.run(
            &self.zip_bin,
            &[
                "-j".to_string(), // flatten paths — a cbz is a flat archive of pages
                "-r".to_string(),
                output.to_string_lossy().into_owned(),
                format!("{}/", dir.display()),
            ],
        )
        .await
    }

    async fn run(&self, bin: &str, args: &[String]) -> Result<()> {
        info!(bin, ?args, "running conversion");

        let status = Command::new(bin)
            .args(args)
            .status()
            .await
            .with_context(|| format!("failed to spawn `{bin}` — is it installed and on PATH?"))?;

        if !status.success() {
            bail!("`{bin}` exited with {status}");
        }

        Ok(())
    }
}

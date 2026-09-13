pub mod convert;

use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceFormat {
    Cbz,
    Cbr,
    Zip,
    Rar,
    Pdf,
    Epub,
    Mobi,
    Azw3,
    ImageFolder,
    Unknown,
}

const IMAGE_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "webp", "gif", "bmp", "avif"];

/// Best-effort sniff of what a torrent's downloaded content actually is,
/// based on file extension (for files) or contents (for directories).
pub fn detect(path: &Path) -> SourceFormat {
    if path.is_dir() {
        return if is_image_folder(path) {
            SourceFormat::ImageFolder
        } else {
            SourceFormat::Unknown
        };
    }

    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .as_deref()
    {
        Some("cbz") => SourceFormat::Cbz,
        Some("cbr") => SourceFormat::Cbr,
        Some("zip") => SourceFormat::Zip,
        Some("rar") => SourceFormat::Rar,
        Some("pdf") => SourceFormat::Pdf,
        Some("epub") => SourceFormat::Epub,
        Some("mobi") => SourceFormat::Mobi,
        Some("azw3") => SourceFormat::Azw3,
        _ => SourceFormat::Unknown,
    }
}

/// A shallow check: true if the directory contains only image files (and at
/// least one). Doesn't recurse — a folder containing per-chapter
/// subdirectories reports `Unknown` here and gets walked one level down by
/// the caller instead. Deeper nesting (e.g. per-volume folders full of
/// per-chapter folders) isn't handled yet.
fn is_image_folder(dir: &Path) -> bool {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return false,
    };

    let mut found_any = false;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            return false;
        }
        match path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
        {
            Some(ext) if IMAGE_EXTENSIONS.contains(&ext.as_str()) => found_any = true,
            _ => return false,
        }
    }
    found_any
}

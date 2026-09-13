use anyhow::{Context, Result};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use zip::{write::SimpleFileOptions, ZipWriter};

pub struct CbzMeta<'a> {
    pub title: &'a str,
    pub volume: Option<&'a str>,
    pub chapter: Option<&'a str>,
    pub series: &'a str,
}

/// Writes all files in `pages_dir` into a CBZ archive at `dest_path`.
pub fn create(pages: &[PathBuf], dest_path: &Path, meta: &CbzMeta) -> Result<()> {
    if let Some(parent) = dest_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create dirs {}", parent.display()))?;
    }

    let file = std::fs::File::create(dest_path)
        .with_context(|| format!("create cbz {}", dest_path.display()))?;
    let mut zip = ZipWriter::new(file);
    let opts = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);

    // ComicInfo.xml
    let xml = comic_info_xml(meta);
    zip.start_file("ComicInfo.xml", opts)?;
    zip.write_all(xml.as_bytes())?;

    for page_path in pages {
        let filename = page_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("page.jpg");
        zip.start_file(filename, opts)?;
        let data = std::fs::read(page_path)
            .with_context(|| format!("read page {}", page_path.display()))?;
        zip.write_all(&data)?;
    }

    zip.finish()?;
    Ok(())
}

/// Build the CBZ filename: `{Title} c{chapter:04} (v{vol}).cbz`
///
/// Chapter number is always the leading sort key so files order correctly
/// regardless of whether a volume is known (MangaDex often leaves `volume`
/// null on recent chapters while backfilling it for older ones — putting
/// volume before chapter in the name would make presence/absence of volume
/// data outrank chapter number in a plain filename sort).
pub fn cbz_filename(title: &str, volume: Option<&str>, chapter: Option<&str>) -> String {
    let mut name = title.to_string();
    let padded = match chapter {
        Some(c) => pad_chapter_num(c),
        None => "0000".to_string(),
    };
    name.push_str(&format!(" c{padded}"));
    if let Some(v) = volume {
        name.push_str(&format!(" (v{v})"));
    }
    name.push_str(".cbz");
    name
}

/// Build the filename for a torrent-sourced whole-volume backfill:
/// `{Title} V{first:04}-{last:04} (v{vol}).cbz`.
///
/// Uses an uppercase `V` as the leading token specifically so a bound volume
/// always sorts before any individually-downloaded chapter of the same
/// series (uppercase letters sort before lowercase in a plain filename
/// comparison, and per-chapter files always lead with lowercase `c` — see
/// `cbz_filename`). A series that has both acquired whole volumes and loose
/// individual chapters should present as "volume 1 ... volume N, then
/// chapter Y ... chapter Z" — a bound volume covering chapters 61-75 isn't
/// meant to interleave with a same-numbered individual chapter download, it
/// supersedes it. Chapter-range digits are still zero-padded (and this is
/// still keyed by the volume's first chapter) purely so multiple volumes of
/// the same series sort correctly relative to each other.
pub fn bulk_volume_filename(title: &str, volume: &str, first_chapter: &str, last_chapter: &str) -> String {
    let mut name = title.to_string();
    name.push_str(&format!(
        " V{}-{}",
        pad_chapter_num(first_chapter),
        pad_chapter_num(last_chapter)
    ));
    name.push_str(&format!(" (v{volume})"));
    name.push_str(".cbz");
    name
}

/// Pads the integer part of a chapter number to 4 digits so integer and
/// fractional chapters compare correctly as plain strings (e.g. chapter 10
/// must sort after chapter 5.5, which requires both to share the same
/// integer-part width).
///
/// MangaDex sometimes gives split-chapter numbers like "37a" or "12b" (also
/// occasionally multi-segment decimals like "12.1.5"), which don't parse as
/// a plain float. Left unpadded, "37a" sorts as a 2-character string next to
/// 4-digit-padded neighbors — e.g. after "c0100" instead of near "c0037" —
/// which is exactly the misordering this padding exists to prevent. So the
/// fallback pads just the leading digit run and keeps whatever follows
/// (letter suffix, extra decimal segments, ...) verbatim.
pub(crate) fn pad_chapter_num(c: &str) -> String {
    if let Ok(n) = c.parse::<f64>() {
        if c.contains('.') {
            return format!("{:08.3}", n);
        }
        return format!("{:04}", n as u64);
    }
    let digits = c.chars().take_while(|ch| ch.is_ascii_digit()).count();
    if digits > 0 {
        if let Ok(n) = c[..digits].parse::<u64>() {
            return format!("{:04}{}", n, &c[digits..]);
        }
    }
    c.to_string()
}

/// Sets (or replaces) `<AlternateSeries>` in an existing CBZ's ComicInfo.xml
/// to `japanese_name`, rewriting every other entry byte-for-byte unchanged.
/// Returns `Ok(false)` if there's no ComicInfo.xml to update, or it already
/// has the desired value (a no-op either way). With `apply: false`, reads
/// and reports what *would* change without writing anything — used for dry
/// runs, since checking still requires opening the archive.
pub fn set_alternate_series(cbz_path: &Path, japanese_name: &str, apply: bool) -> Result<bool> {
    let file = std::fs::File::open(cbz_path)
        .with_context(|| format!("open {}", cbz_path.display()))?;
    let mut archive =
        zip::ZipArchive::new(file).with_context(|| format!("read zip {}", cbz_path.display()))?;

    let mut old_xml = String::new();
    match archive.by_name("ComicInfo.xml") {
        Ok(mut f) => f
            .read_to_string(&mut old_xml)
            .with_context(|| format!("read ComicInfo.xml in {}", cbz_path.display()))?,
        Err(_) => return Ok(false),
    };

    let new_xml = upsert_alternate_series(&old_xml, japanese_name);
    if new_xml == old_xml {
        return Ok(false);
    }
    if !apply {
        return Ok(true);
    }

    let tmp_path = cbz_path.with_extension("cbz.tmp");
    {
        let tmp_file = std::fs::File::create(&tmp_path)
            .with_context(|| format!("create {}", tmp_path.display()))?;
        let mut writer = ZipWriter::new(tmp_file);
        let opts = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);

        for i in 0..archive.len() {
            let mut entry = archive.by_index(i)?;
            let name = entry.name().to_string();
            writer.start_file(&name, opts)?;
            if name == "ComicInfo.xml" {
                writer.write_all(new_xml.as_bytes())?;
            } else {
                let mut buf = Vec::with_capacity(entry.size() as usize);
                entry.read_to_end(&mut buf)?;
                writer.write_all(&buf)?;
            }
        }
        writer.finish()?;
    }
    std::fs::rename(&tmp_path, cbz_path)
        .with_context(|| format!("replace {}", cbz_path.display()))?;
    Ok(true)
}

fn upsert_alternate_series(xml: &str, japanese_name: &str) -> String {
    let escaped = xml_escape(japanese_name);
    let re = regex::Regex::new(r"(?s)\s*<AlternateSeries>.*?</AlternateSeries>").expect("valid regex");
    let stripped = re.replace(xml, "").into_owned();
    let insertion = format!("  <AlternateSeries>{escaped}</AlternateSeries>\n</ComicInfo>");
    stripped.replacen("</ComicInfo>", &insertion, 1)
}

fn comic_info_xml(meta: &CbzMeta) -> String {
    let series = xml_escape(meta.series);
    let title = xml_escape(meta.title);
    let volume = meta.volume.map(xml_escape).unwrap_or_default();
    let number = meta.chapter.map(xml_escape).unwrap_or_default();
    format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<ComicInfo xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
           xmlns:xsd="http://www.w3.org/2001/XMLSchema">
  <Series>{series}</Series>
  <Title>{title}</Title>
  <Volume>{volume}</Volume>
  <Number>{number}</Number>
  <Manga>YesAndRightToLeft</Manga>
</ComicInfo>
"#
    )
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

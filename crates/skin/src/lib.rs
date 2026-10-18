//! Classic (2.x) skin loading: `.wsz` archives and plain skin folders.
//!
//! Every bitmap is decoded and re-encoded as PNG so the UI can use it directly.
//! Sheets the skin does not provide fall back to the built-in default skin.

use std::collections::HashMap;
use std::io::{Cursor, Read, Seek};
use std::path::Path;

use serde::Serialize;

mod cursor;
mod default_skin;
mod region;
mod text;

pub use cursor::{CursorInfo, SkinCursor};
pub use region::Regions;
pub use text::{PlaylistStyle, VisColors};

/// Bitmap sheets used by the classic UI, by lowercase base name (without extension).
pub const SHEETS: &[&str] = &[
    "main", "cbuttons", "titlebar", "shufrep", "text", "volume", "balance", "monoster",
    "playpaus", "numbers", "nums_ex", "posbar", "eqmain", "eq_ex", "pledit", "gen", "genex",
];

#[derive(Debug, thiserror::Error)]
pub enum SkinError {
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid skin archive: {0}")]
    Archive(#[from] zip::result::ZipError),
    #[error("skin has no usable bitmaps")]
    Empty,
}

#[derive(Debug, Clone)]
pub struct Skin {
    /// Display name (archive or folder name).
    pub name: String,
    /// PNG-encoded sheets keyed by base name, e.g. `"main"`.
    pub sheets: HashMap<String, Vec<u8>>,
    pub viscolors: VisColors,
    pub playlist: PlaylistStyle,
    /// Cursors keyed by lowercase base name, e.g. `"titlebar"`. Never taken from the
    /// fallback: without them the UI keeps the system cursors.
    pub cursors: HashMap<String, SkinCursor>,
    /// Window shapes from `region.txt` (empty when the skin has none).
    pub regions: Regions,
}

/// What the UI needs to know about the active skin (images come via the skin protocol).
#[derive(Debug, Clone, Serialize)]
pub struct SkinInfo {
    pub name: String,
    pub sheets: Vec<String>,
    pub viscolors: VisColors,
    pub playlist: PlaylistStyle,
    /// True when the skin ships `nums_ex.bmp`, which then replaces `numbers.bmp`.
    pub has_nums_ex: bool,
    pub cursors: HashMap<String, CursorInfo>,
    pub regions: Regions,
}

impl Skin {
    /// The built-in skin. Always complete.
    pub fn default_skin() -> Self {
        let files = default_skin::FILES.iter().map(|(n, b)| (n.to_string(), b.to_vec()));
        Self::from_files("Kyara Default", files, None).expect("built-in skin is valid")
    }

    /// Load a `.wsz`/`.zip` archive or a directory containing skin files.
    pub fn load(path: &Path) -> Result<Self, SkinError> {
        let name = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "skin".into());
        let files = if path.is_dir() { read_dir(path)? } else { read_zip(std::fs::File::open(path)?)? };
        Self::from_files(&name, files, Some(&Self::default_skin()))
    }

    /// Load from an in-memory archive.
    pub fn from_zip_bytes(name: &str, bytes: &[u8]) -> Result<Self, SkinError> {
        Self::from_files(name, read_zip(Cursor::new(bytes))?, Some(&Self::default_skin()))
    }

    fn from_files(
        name: &str,
        files: impl IntoIterator<Item = (String, Vec<u8>)>,
        fallback: Option<&Skin>,
    ) -> Result<Self, SkinError> {
        // Index by lowercase base name. Skins are often zipped with a parent folder,
        // so ignore directories; the shallowest entry wins on duplicates.
        let mut index: HashMap<String, (usize, Vec<u8>)> = HashMap::new();
        for (path, data) in files {
            let norm = path.replace('\\', "/");
            let depth = norm.matches('/').count();
            let file = norm.rsplit('/').next().unwrap_or(&norm).to_ascii_lowercase();
            if index.get(&file).is_none_or(|(d, _)| depth < *d) {
                index.insert(file, (depth, data));
            }
        }
        let get = |file: &str| index.get(file).map(|(_, d)| d.as_slice());

        let mut sheets = HashMap::new();
        for &sheet in SHEETS {
            let found = get(&format!("{sheet}.bmp")).or_else(|| get(&format!("{sheet}.png")));
            let Some(data) = found else { continue };
            match to_png(data) {
                Ok(png) => {
                    sheets.insert(sheet.to_string(), png);
                }
                Err(e) => log::warn!("skin {name}: cannot decode {sheet}: {e}"),
            }
        }
        if sheets.is_empty() {
            return Err(SkinError::Empty);
        }
        // Winamp uses the volume bitmap when a skin has no balance bitmap.
        if !sheets.contains_key("balance") {
            if let Some(v) = sheets.get("volume").cloned() {
                sheets.insert("balance".into(), v);
            }
        }

        let viscolors = get("viscolor.txt")
            .map(|d| VisColors::parse(&String::from_utf8_lossy(d)))
            .or_else(|| fallback.map(|f| f.viscolors.clone()))
            .unwrap_or_default();
        let playlist = get("pledit.txt")
            .map(|d| PlaylistStyle::parse(&String::from_utf8_lossy(d)))
            .or_else(|| fallback.map(|f| f.playlist.clone()))
            .unwrap_or_default();

        let mut cursors = HashMap::new();
        for (file, (_, data)) in &index {
            let Some(base) = file.strip_suffix(".cur") else { continue };
            match cursor::decode(data) {
                Some(c) => {
                    cursors.insert(base.to_string(), c);
                }
                None => log::warn!("skin {name}: cannot decode cursor {file}"),
            }
        }
        let regions = get("region.txt").map(|d| region::parse(&String::from_utf8_lossy(d))).unwrap_or_default();

        let mut skin = Self { name: name.to_string(), sheets, viscolors, playlist, cursors, regions };
        if let Some(fallback) = fallback {
            for (k, v) in &fallback.sheets {
                // nums_ex is optional by design; a fallback copy would override numbers.bmp.
                if k != "nums_ex" {
                    skin.sheets.entry(k.clone()).or_insert_with(|| v.clone());
                }
            }
        }
        Ok(skin)
    }

    pub fn info(&self) -> SkinInfo {
        let mut sheets: Vec<String> = self.sheets.keys().cloned().collect();
        sheets.sort();
        SkinInfo {
            name: self.name.clone(),
            has_nums_ex: self.sheets.contains_key("nums_ex"),
            sheets,
            viscolors: self.viscolors.clone(),
            playlist: self.playlist.clone(),
            cursors: self
                .cursors
                .iter()
                .map(|(k, c)| (k.clone(), CursorInfo { x: c.hotspot.0, y: c.hotspot.1 }))
                .collect(),
            regions: self.regions.clone(),
        }
    }

    /// PNG bytes of cursor `name` (`"titlebar"`, case-insensitive).
    pub fn cursor(&self, name: &str) -> Option<&[u8]> {
        self.cursors.get(&name.to_ascii_lowercase()).map(|c| c.png.as_slice())
    }

    /// PNG bytes for `sheet`, accepting `"main"`, `"main.png"` or `"MAIN.BMP"`.
    pub fn sheet(&self, sheet: &str) -> Option<&[u8]> {
        let lower = sheet.to_ascii_lowercase();
        let base = lower.rsplit_once('.').map_or(lower.as_str(), |(b, _)| b);
        self.sheets.get(base).map(Vec::as_slice)
    }
}

fn to_png(data: &[u8]) -> Result<Vec<u8>, image::ImageError> {
    let img = match image::load_from_memory(data) {
        Ok(img) => img,
        Err(e) => match repair_bmp_offset(data) {
            Some(fixed) => image::load_from_memory(&fixed)?,
            None => return Err(e),
        },
    };
    let mut out = Vec::new();
    img.to_rgba8().write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png)?;
    Ok(out)
}

/// Some BMPs carry a bogus pixel-data offset (`bfOffBits`). Windows ignores it for
/// packed bitmaps, so recompute it from the header and palette size.
fn repair_bmp_offset(data: &[u8]) -> Option<Vec<u8>> {
    let u16_at = |o: usize| data.get(o..o + 2).map(|b| u16::from_le_bytes([b[0], b[1]]));
    let u32_at = |o: usize| data.get(o..o + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    if data.get(..2)? != b"BM" {
        return None;
    }
    let dib_size = u32_at(14)?;
    let bpp = u16_at(28)?;
    let compression = u32_at(30)?;
    let colors_used = u32_at(46)?;
    let palette = match (colors_used, bpp) {
        (0, b) if b <= 8 => 1u32 << b,
        (n, _) => n,
    };
    let bitfields = if dib_size == 40 && compression == 3 { 12 } else { 0 };
    let offset = 14 + dib_size + bitfields + palette * 4;
    if offset as usize >= data.len() || u32_at(10)? == offset {
        return None;
    }
    let mut fixed = data.to_vec();
    fixed[10..14].copy_from_slice(&offset.to_le_bytes());
    Some(fixed)
}

fn read_zip<R: Read + Seek>(reader: R) -> Result<Vec<(String, Vec<u8>)>, SkinError> {
    let mut archive = zip::ZipArchive::new(reader)?;
    let mut files = Vec::new();
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        if entry.is_dir() {
            continue;
        }
        let name = match entry.name() {
            Ok(n) => n.into_owned(),
            Err(_) => continue,
        };
        // Skin files are small; skip anything absurd instead of exhausting memory.
        if entry.size() > 16 * 1024 * 1024 {
            continue;
        }
        let mut data = Vec::with_capacity(entry.size() as usize);
        entry.read_to_end(&mut data)?;
        files.push((name, data));
    }
    Ok(files)
}

fn read_dir(dir: &Path) -> Result<Vec<(String, Vec<u8>)>, SkinError> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        if entry.file_type()?.is_file() {
            files.push((entry.file_name().to_string_lossy().into_owned(), std::fs::read(entry.path())?));
        }
    }
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn bmp(w: u32, h: u32) -> Vec<u8> {
        let img = image::RgbImage::from_pixel(w, h, image::Rgb([255, 0, 0]));
        let mut out = Vec::new();
        img.write_to(&mut Cursor::new(&mut out), image::ImageFormat::Bmp).unwrap();
        out
    }

    fn wsz(entries: &[(&str, Vec<u8>)]) -> Vec<u8> {
        let mut buf = Cursor::new(Vec::new());
        let mut zip = zip::ZipWriter::new(&mut buf);
        let opts = zip::write::SimpleFileOptions::default();
        for (name, data) in entries {
            zip.start_file(*name, opts).unwrap();
            zip.write_all(data).unwrap();
        }
        zip.finish().unwrap();
        buf.into_inner()
    }

    #[test]
    fn default_skin_is_complete() {
        let skin = Skin::default_skin();
        for sheet in SHEETS.iter().filter(|s| **s != "nums_ex") {
            assert!(skin.sheets.contains_key(*sheet), "missing {sheet}");
        }
        let main = image::load_from_memory(skin.sheet("main").unwrap()).unwrap();
        assert_eq!((main.width(), main.height()), (275, 116));
    }

    #[test]
    fn loads_nested_case_insensitive_and_falls_back() {
        let bytes = wsz(&[
            ("MySkin/MAIN.BMP", bmp(275, 116)),
            ("MySkin/Volume.bmp", bmp(68, 433)),
            ("MySkin/viscolor.txt", b"1,2,3\n".to_vec()),
            ("MySkin/TitleBar.CUR", cursor::tests::cur_file((3, 4))),
            ("MySkin/region.txt", b"[Normal]\nNumPoints=3\nPointList=0,0,9,0,0,9\n".to_vec()),
        ]);
        let skin = Skin::from_zip_bytes("MySkin", &bytes).unwrap();
        // Red main sheet comes from the archive.
        let main = image::load_from_memory(skin.sheet("main.bmp").unwrap()).unwrap().to_rgba8();
        assert_eq!(main.get_pixel(0, 0).0, [255, 0, 0, 255]);
        // Balance falls back to the archive's volume, not to the default skin.
        assert_eq!(skin.sheet("balance"), skin.sheet("volume"));
        // Missing sheets come from the default skin; nums_ex stays absent.
        assert!(skin.sheet("cbuttons").is_some());
        assert!(!skin.info().has_nums_ex);
        assert_eq!(skin.viscolors.0[0], [1, 2, 3]);
        assert!(skin.cursor("titlebar").is_some());
        assert_eq!(skin.info().cursors["titlebar"].y, 4);
        assert_eq!(skin.regions["normal"][0].len(), 3);
        // The default skin has neither cursors nor regions.
        assert!(Skin::default_skin().cursors.is_empty());
    }

    #[test]
    fn repairs_bogus_pixel_offset() {
        let img = image::GrayImage::from_fn(8, 4, |x, _| image::Luma([x as u8 * 30]));
        let mut data = Vec::new();
        image::DynamicImage::ImageLuma8(img)
            .to_rgb8()
            .write_to(&mut Cursor::new(&mut data), image::ImageFormat::Bmp)
            .unwrap();
        data[10..14].copy_from_slice(&0xFFFF_EE00u32.to_le_bytes());
        let png = to_png(&data).expect("decodes after repair");
        let out = image::load_from_memory(&png).unwrap().to_rgba8();
        assert_eq!(out.get_pixel(7, 0).0, [210, 210, 210, 255]);
    }

    #[test]
    fn rejects_archives_without_bitmaps() {
        let bytes = wsz(&[("readme.txt", b"hi".to_vec())]);
        assert!(matches!(Skin::from_zip_bytes("x", &bytes), Err(SkinError::Empty)));
    }
}

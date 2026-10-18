//! Playlist files: M3U/M3U8 (with `#EXTINF`) and PLS.

use std::io;
use std::path::{Path, PathBuf};

use crate::Entry;

#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub path: PathBuf,
    pub title: Option<String>,
    pub duration: Option<f64>,
}

pub fn read(path: &Path) -> io::Result<Vec<Item>> {
    let bytes = std::fs::read(path)?;
    // M3U is traditionally in the system codepage; fall back to Latin-1 when not UTF-8.
    let text = match String::from_utf8(bytes) {
        Ok(s) => s,
        Err(e) => e.into_bytes().iter().map(|&b| b as char).collect(),
    };
    let base = path.parent().unwrap_or(Path::new("."));
    let is_pls = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("pls"));
    Ok(if is_pls { parse_pls(&text, base) } else { parse_m3u(&text, base) })
}

/// Resolve a playlist line relative to the playlist's folder. URLs are kept as-is.
fn resolve(base: &Path, raw: &str) -> PathBuf {
    let raw = raw.trim();
    if let Some(rest) = raw.strip_prefix("file://") {
        return PathBuf::from(percent_decode(rest));
    }
    if raw.contains("://") {
        return PathBuf::from(raw);
    }
    let normalized = if cfg!(windows) { raw.to_string() } else { raw.replace('\\', "/") };
    let p = PathBuf::from(normalized);
    if p.is_absolute() { p } else { base.join(p) }
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
            if let Some(b) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn parse_m3u(text: &str, base: &Path) -> Vec<Item> {
    let mut items = Vec::new();
    let mut pending: Option<(Option<f64>, Option<String>)> = None;
    for line in text.lines() {
        let line = line.trim().trim_start_matches('\u{feff}');
        if line.is_empty() {
            continue;
        }
        if let Some(info) = line.strip_prefix("#EXTINF:") {
            let (dur, title) = info.split_once(',').unwrap_or((info, ""));
            let dur = dur.trim().parse::<f64>().ok().filter(|d| *d >= 0.0);
            let title = Some(title.trim().to_string()).filter(|t| !t.is_empty());
            pending = Some((dur, title));
            continue;
        }
        if line.starts_with('#') {
            continue;
        }
        let (duration, title) = pending.take().unwrap_or((None, None));
        items.push(Item { path: resolve(base, line), title, duration });
    }
    items
}

pub fn parse_pls(text: &str, base: &Path) -> Vec<Item> {
    use std::collections::BTreeMap;
    let mut slots: BTreeMap<u32, Item> = BTreeMap::new();
    for line in text.lines() {
        let Some((key, value)) = line.trim().split_once('=') else { continue };
        let key = key.trim().to_ascii_lowercase();
        let (field, num) = key.split_at(key.find(|c: char| c.is_ascii_digit()).unwrap_or(key.len()));
        let Ok(n) = num.parse::<u32>() else { continue };
        let slot = slots
            .entry(n)
            .or_insert_with(|| Item { path: PathBuf::new(), title: None, duration: None });
        match field {
            "file" => slot.path = resolve(base, value),
            "title" => slot.title = Some(value.trim().to_string()).filter(|t| !t.is_empty()),
            "length" => slot.duration = value.trim().parse::<f64>().ok().filter(|d| *d >= 0.0),
            _ => {}
        }
    }
    slots.into_values().filter(|i| !i.path.as_os_str().is_empty()).collect()
}

/// Write an extended M3U (UTF-8) with absolute paths.
pub fn write_m3u8(path: &Path, entries: &[Entry]) -> io::Result<()> {
    let mut out = String::from("#EXTM3U\n");
    for e in entries {
        let dur = e.duration.map_or(-1, |d| d.round() as i64);
        out.push_str(&format!("#EXTINF:{dur},{}\n{}\n", e.title, e.path.display()));
    }
    std::fs::write(path, out)
}

/// Write a PLS playlist.
pub fn write_pls(path: &Path, entries: &[Entry]) -> io::Result<()> {
    let mut out = String::from("[playlist]\n");
    for (i, e) in entries.iter().enumerate() {
        let n = i + 1;
        let dur = e.duration.map_or(-1, |d| d.round() as i64);
        out.push_str(&format!("File{n}={}\nTitle{n}={}\nLength{n}={dur}\n", e.path.display(), e.title));
    }
    out.push_str(&format!("NumberOfEntries={}\nVersion=2\n", entries.len()));
    std::fs::write(path, out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn m3u_extinf_and_relative_paths() {
        let text = "#EXTM3U\n#EXTINF:123,Artist - Song\nsub/a.mp3\n\n/abs/b.flac\nhttp://radio:8000/stream\nfile:///x/y%20z.ogg\n";
        let items = parse_m3u(text, Path::new("/lists"));
        assert_eq!(items.len(), 4);
        assert_eq!(items[0], Item { path: "/lists/sub/a.mp3".into(), title: Some("Artist - Song".into()), duration: Some(123.0) });
        assert_eq!(items[1].path, PathBuf::from("/abs/b.flac"));
        assert_eq!(items[1].title, None);
        assert_eq!(items[2].path, PathBuf::from("http://radio:8000/stream"));
        assert_eq!(items[3].path, PathBuf::from("/x/y z.ogg"));
    }

    #[test]
    fn pls_slots_in_numeric_order() {
        let text = "[playlist]\nFile2=b.mp3\nTitle2=B\nFile1=a.mp3\nLength1=-1\nLength2=61\nNumberOfEntries=2\n";
        let items = parse_pls(text, Path::new("/p"));
        assert_eq!(items[0].path, PathBuf::from("/p/a.mp3"));
        assert_eq!(items[0].duration, None);
        assert_eq!(items[1].title.as_deref(), Some("B"));
        assert_eq!(items[1].duration, Some(61.0));
    }

    #[test]
    fn m3u8_round_trip() {
        let dir = std::env::temp_dir().join(format!("kyara-pl-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("t.m3u8");
        let entries = vec![Entry {
            id: 1,
            path: "/música/ação.mp3".into(),
            title: "Ação".into(),
            duration: Some(5.4),
            meta_loaded: true,
        }];
        write_m3u8(&file, &entries).unwrap();
        let items = read(&file).unwrap();
        assert_eq!(items[0].path, PathBuf::from("/música/ação.mp3"));
        assert_eq!(items[0].title.as_deref(), Some("Ação"));
        assert_eq!(items[0].duration, Some(5.0));
        std::fs::remove_dir_all(dir).unwrap();
    }
}

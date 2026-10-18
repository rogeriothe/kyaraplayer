//! Tag reading for playlist titles.

use std::path::Path;

use lofty::prelude::*;
use lofty::tag::ItemKey;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Meta {
    pub artist: Option<String>,
    pub title: Option<String>,
    pub duration: Option<f64>,
}

impl Meta {
    /// Classic title format: "Artist - Title", falling back to whatever is known.
    pub fn display_title(&self) -> Option<String> {
        match (&self.artist, &self.title) {
            (Some(a), Some(t)) => Some(format!("{a} - {t}")),
            (None, Some(t)) => Some(t.clone()),
            _ => None,
        }
    }
}

pub fn read(path: &Path) -> Option<Meta> {
    let file = lofty::read_from_path(path).ok()?;
    let clean = |s: std::borrow::Cow<'_, str>| Some(s.trim().to_string()).filter(|s| !s.is_empty());
    let tag = file.primary_tag().or_else(|| file.first_tag());
    let duration = file.properties().duration().as_secs_f64();
    Some(Meta {
        artist: tag.and_then(|t| t.artist()).and_then(clean),
        title: tag.and_then(|t| t.title()).and_then(clean),
        duration: (duration > 0.0).then_some(duration),
    })
}

/// Editable tag fields, as shown in the file info window. Empty means "not set".
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Tags {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub year: String,
    pub genre: String,
    pub track: String,
    pub comment: String,
}

/// Everything the file info window shows.
#[derive(Debug, Clone, Serialize)]
pub struct Details {
    pub tags: Tags,
    pub format: String,
    pub duration: Option<f64>,
    /// Audio bitrate in kbps.
    pub bitrate: Option<u32>,
    pub sample_rate: Option<u32>,
    pub channels: Option<u8>,
    pub bit_depth: Option<u8>,
    pub file_size: Option<u64>,
    /// Name of the tag format edits are written to (ID3v2, Vorbis comments, ...).
    pub tag_format: String,
}

fn read_file(path: &Path) -> Result<lofty::file::TaggedFile, String> {
    lofty::read_from_path(path).map_err(|e| e.to_string())
}

pub fn details(path: &Path) -> Result<Details, String> {
    let file = read_file(path)?;
    let props = file.properties();
    let tag = file.primary_tag().or_else(|| file.first_tag());
    let text = |key: ItemKey| tag.and_then(|t| t.get_string(key)).unwrap_or_default().trim().to_string();
    let tags = Tags {
        title: text(ItemKey::TrackTitle),
        artist: text(ItemKey::TrackArtist),
        album: text(ItemKey::AlbumTitle),
        year: Some(text(ItemKey::RecordingDate)).filter(|s| !s.is_empty()).unwrap_or_else(|| text(ItemKey::Year)),
        genre: text(ItemKey::Genre),
        track: text(ItemKey::TrackNumber),
        comment: text(ItemKey::Comment),
    };
    let duration = props.duration().as_secs_f64();
    Ok(Details {
        tags,
        format: format!("{:?}", file.file_type()),
        duration: (duration > 0.0).then_some(duration),
        bitrate: props.audio_bitrate().or(props.overall_bitrate()),
        sample_rate: props.sample_rate(),
        channels: props.channels(),
        bit_depth: props.bit_depth(),
        file_size: std::fs::metadata(path).ok().map(|m| m.len()),
        tag_format: format!("{:?}", file.primary_tag_type()),
    })
}

/// Write `tags` into the file's primary tag, creating it if needed.
pub fn write_tags(path: &Path, tags: &Tags) -> Result<(), String> {
    use lofty::config::WriteOptions;
    use lofty::tag::Tag;

    let mut file = read_file(path)?;
    let tag_type = file.primary_tag_type();
    if file.primary_tag().is_none() {
        file.insert_tag(Tag::new(tag_type));
    }
    let tag = file.primary_tag_mut().ok_or("this format has no writable tag")?;
    let mut set = |key: ItemKey, value: &str| {
        let value = value.trim();
        if value.is_empty() {
            tag.remove_key(key);
        } else {
            tag.insert_text(key, value.to_string());
        }
    };
    set(ItemKey::TrackTitle, &tags.title);
    set(ItemKey::TrackArtist, &tags.artist);
    set(ItemKey::AlbumTitle, &tags.album);
    set(ItemKey::Genre, &tags.genre);
    set(ItemKey::TrackNumber, &tags.track);
    set(ItemKey::Comment, &tags.comment);
    set(ItemKey::Year, "");
    set(ItemKey::RecordingDate, &tags.year);
    file.save_to_path(path, WriteOptions::default()).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One second of 8 kHz mono silence.
    fn wav() -> Vec<u8> {
        let data = vec![0u8; 16_000];
        let mut v = Vec::new();
        v.extend_from_slice(b"RIFF");
        v.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
        v.extend_from_slice(b"WAVEfmt ");
        v.extend_from_slice(&16u32.to_le_bytes());
        v.extend_from_slice(&[1, 0, 1, 0]); // PCM, mono
        v.extend_from_slice(&8000u32.to_le_bytes());
        v.extend_from_slice(&16000u32.to_le_bytes());
        v.extend_from_slice(&[2, 0, 16, 0]);
        v.extend_from_slice(b"data");
        v.extend_from_slice(&(data.len() as u32).to_le_bytes());
        v.extend_from_slice(&data);
        v
    }

    #[test]
    fn reads_flac_vorbis_comments() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../audio-engine/tests/fixtures/sine.flac");
        let m = read(&path).unwrap();
        assert_eq!(m.display_title().as_deref(), Some("Kyara - Fixture"));
        assert!(m.duration.is_some_and(|d| (d - 0.5).abs() < 0.01));
        let d = details(&path).unwrap();
        assert_eq!((d.sample_rate, d.channels, d.bit_depth), (Some(44_100), Some(2), Some(16)));
        assert_eq!(d.tag_format, "VorbisComments");
    }

    #[test]
    fn tags_round_trip() {
        let path = std::env::temp_dir().join(format!("kyara-tags-{}.wav", std::process::id()));
        std::fs::write(&path, wav()).unwrap();
        let tags = Tags { title: "Título".into(), artist: "Artista".into(), ..Default::default() };
        write_tags(&path, &tags).unwrap();
        let d = details(&path).unwrap();
        std::fs::remove_file(&path).ok();
        assert_eq!((d.tags.title.as_str(), d.tags.artist.as_str()), ("Título", "Artista"));
        assert_eq!((d.sample_rate, d.channels), (Some(8000), Some(1)));
        assert!(d.duration.is_some_and(|s| (s - 1.0).abs() < 0.01));
    }
}

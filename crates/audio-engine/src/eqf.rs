//! Classic equalizer preset files (`.eqf` single preset, `.q1` library).
//!
//! Layout: a 31-byte header, then per preset a 257-byte NUL-padded name and 11
//! bytes: ten band values then the preamp, each 0..=63 where 0 is +12 dB and 63 is -12 dB.

use serde::{Deserialize, Serialize};

use crate::eq::{BANDS, MAX_DB};

const HEADER: &[u8; 31] = b"Winamp EQ library file v1.1\x1a!--";
const NAME_LEN: usize = 257;
const RECORD_LEN: usize = NAME_LEN + BANDS + 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Preset {
    pub name: String,
    pub preamp_db: f32,
    pub bands_db: [f32; BANDS],
}

fn to_db(v: u8) -> f32 {
    let v = v.min(63) as f32;
    ((31.5 - v) / 31.5 * MAX_DB * 10.0).round() / 10.0
}

fn from_db(db: f32) -> u8 {
    (31.5 - db.clamp(-MAX_DB, MAX_DB) / MAX_DB * 31.5).round().clamp(0.0, 63.0) as u8
}

pub fn parse(bytes: &[u8]) -> Option<Vec<Preset>> {
    let body = bytes.strip_prefix(HEADER.as_slice())?;
    let presets = body
        .chunks_exact(RECORD_LEN)
        .map(|rec| {
            let name_bytes = &rec[..NAME_LEN];
            let end = name_bytes.iter().position(|&b| b == 0).unwrap_or(NAME_LEN);
            // Names are in the Windows codepage; Latin-1 is the closest portable guess.
            let name = name_bytes[..end].iter().map(|&b| b as char).collect();
            let vals = &rec[NAME_LEN..];
            Preset {
                name,
                bands_db: std::array::from_fn(|i| to_db(vals[i])),
                preamp_db: to_db(vals[BANDS]),
            }
        })
        .collect();
    Some(presets)
}

pub fn serialize(presets: &[Preset]) -> Vec<u8> {
    let mut out = HEADER.to_vec();
    for p in presets {
        let mut name = [0u8; NAME_LEN];
        let latin1: Vec<u8> = p.name.chars().map(|c| if (c as u32) < 256 { c as u8 } else { b'?' }).collect();
        let n = latin1.len().min(NAME_LEN - 1);
        name[..n].copy_from_slice(&latin1[..n]);
        out.extend_from_slice(&name);
        out.extend(p.bands_db.iter().map(|&db| from_db(db)));
        out.push(from_db(p.preamp_db));
    }
    out
}

/// Built-in presets (original values).
pub fn builtin() -> Vec<Preset> {
    let p = |name: &str, preamp_db: f32, bands_db: [f32; BANDS]| Preset { name: name.into(), preamp_db, bands_db };
    vec![
        p("Flat", 0.0, [0.0; BANDS]),
        p("Bass Boost", -4.0, [8.0, 7.0, 5.0, 2.5, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]),
        p("Treble Boost", -4.0, [0.0, 0.0, 0.0, 0.0, 0.0, 1.5, 4.0, 6.5, 7.5, 8.0]),
        p("Loudness", -4.0, [6.0, 4.5, 2.0, 0.0, -1.0, 0.0, 1.5, 4.0, 5.0, 5.5]),
        p("Rock", -2.0, [5.0, 3.5, 1.5, -1.5, -2.5, -0.5, 2.5, 4.5, 5.0, 5.0]),
        p("Pop", -1.5, [-1.0, 1.0, 3.0, 4.0, 3.0, 0.5, -1.0, -1.0, -1.0, -1.0]),
        p("Jazz", -1.0, [3.0, 2.0, 0.5, 1.5, -1.0, -1.0, 0.0, 1.5, 2.5, 3.0]),
        p("Classical", 0.0, [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, -3.0, -4.5, -4.5, -6.0]),
        p("Dance", -3.0, [7.0, 5.5, 2.0, 0.0, 0.0, -3.0, -4.0, -4.0, 0.0, 0.0]),
        p("Vocal", -2.0, [-3.0, -2.0, 0.0, 3.0, 4.5, 4.5, 3.0, 1.0, 0.0, -1.0]),
        p("Soft", 0.0, [2.0, 1.0, 0.0, -1.0, -1.0, 0.0, 1.5, 3.0, 3.5, 4.0]),
        p("Headphones", -3.0, [3.5, 5.5, 2.5, -2.0, -1.0, 1.0, 3.0, 5.5, 6.5, 7.0]),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn db_mapping_endpoints() {
        assert_eq!(to_db(0), 12.0);
        assert_eq!(to_db(63), -12.0);
        assert_eq!(from_db(12.0), 0);
        assert_eq!(from_db(-12.0), 63);
        assert!(to_db(from_db(0.0)).abs() <= 0.2);
    }

    #[test]
    fn round_trip() {
        let presets = vec![
            Preset { name: "Mine é".into(), preamp_db: -6.0, bands_db: [12.0, -12.0, 0.0, 3.0, -3.0, 6.0, -6.0, 9.0, -9.0, 0.0] },
            builtin()[1].clone(),
        ];
        let bytes = serialize(&presets);
        assert_eq!(bytes.len(), 31 + 2 * RECORD_LEN);
        let back = parse(&bytes).unwrap();
        assert_eq!(back[0].name, "Mine é");
        for (a, b) in back.iter().zip(&presets) {
            assert!((a.preamp_db - b.preamp_db).abs() <= 0.2);
            for (x, y) in a.bands_db.iter().zip(&b.bands_db) {
                assert!((x - y).abs() <= 0.2, "{x} vs {y}");
            }
        }
    }

    #[test]
    fn rejects_other_files() {
        assert!(parse(b"not an eq file").is_none());
    }
}

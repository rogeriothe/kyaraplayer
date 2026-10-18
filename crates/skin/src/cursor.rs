//! Skin cursors (`*.cur`). Each one is converted to PNG plus its hotspot so the UI
//! can use it as a CSS cursor.

use std::io::Cursor as IoCursor;

use serde::Serialize;

#[derive(Debug, Clone)]
pub struct SkinCursor {
    pub png: Vec<u8>,
    pub hotspot: (u16, u16),
}

/// Hotspot and size the UI needs to build `cursor: url(...) x y`.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct CursorInfo {
    pub x: u16,
    pub y: u16,
}

/// Decode a static Windows cursor. Animated cursors (`RIFF`/ANI) are not supported.
pub fn decode(data: &[u8]) -> Option<SkinCursor> {
    let u16_at = |o: usize| data.get(o..o + 2).map(|b| u16::from_le_bytes([b[0], b[1]]));
    let u32_at = |o: usize| data.get(o..o + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    if u16_at(0)? != 0 || u16_at(2)? != 2 {
        return None;
    }
    let count = u16_at(4)? as usize;
    // Use the largest image; Winamp cursors normally hold a single 32x32 one.
    let entry = (0..count)
        .map(|i| 6 + i * 16)
        .filter(|&e| e + 16 <= data.len())
        .max_by_key(|&e| {
            let side = |b: u8| if b == 0 { 256u32 } else { u32::from(b) };
            side(data[e]) * side(data[e + 1])
        })?;
    let hotspot = (u16_at(entry + 4)?, u16_at(entry + 6)?);
    let len = u32_at(entry + 8)? as usize;
    let offset = u32_at(entry + 12)? as usize;
    let image = data.get(offset..offset.checked_add(len)?)?;

    // Re-wrap that image as a one-entry icon (type 1) for the ICO decoder. The CUR
    // hotspot fields overlap the icon's planes/bit-count fields, so reset them.
    let mut ico = Vec::with_capacity(22 + image.len());
    ico.extend_from_slice(&[0, 0, 1, 0, 1, 0]);
    ico.extend_from_slice(&data[entry..entry + 4]);
    ico.extend_from_slice(&1u16.to_le_bytes());
    ico.extend_from_slice(&0u16.to_le_bytes());
    ico.extend_from_slice(&(image.len() as u32).to_le_bytes());
    ico.extend_from_slice(&22u32.to_le_bytes());
    ico.extend_from_slice(image);

    let img = match image::load_from_memory_with_format(&ico, image::ImageFormat::Ico) {
        Ok(img) => img,
        Err(e) => {
            log::warn!("cannot decode cursor: {e}");
            return None;
        }
    };
    let mut png = Vec::new();
    img.to_rgba8().write_to(&mut IoCursor::new(&mut png), image::ImageFormat::Png).ok()?;
    Some(SkinCursor { png, hotspot })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A 32x32 cursor with a 1-bit image: left half black, right half transparent.
    pub fn cur_file(hotspot: (u16, u16)) -> Vec<u8> {
        let (w, h) = (32u32, 32u32);
        let mut dib = Vec::new();
        dib.extend_from_slice(&40u32.to_le_bytes());
        dib.extend_from_slice(&w.to_le_bytes());
        dib.extend_from_slice(&(h * 2).to_le_bytes()); // XOR + AND masks
        dib.extend_from_slice(&1u16.to_le_bytes());
        dib.extend_from_slice(&1u16.to_le_bytes()); // 1 bpp
        dib.extend_from_slice(&[0; 24]);
        dib.extend_from_slice(&[0, 0, 0, 0, 255, 255, 255, 0]); // palette: black, white
        // XOR mask: all black (index 0). Rows are 4 bytes (32 px).
        dib.extend(std::iter::repeat_n(0u8, (h * 4) as usize));
        // AND mask: 1 = transparent. Right half of each row transparent.
        for _ in 0..h {
            dib.extend_from_slice(&[0x00, 0x00, 0xFF, 0xFF]);
        }
        let mut cur = vec![0, 0, 2, 0, 1, 0, 32, 32, 0, 0];
        cur.extend_from_slice(&hotspot.0.to_le_bytes());
        cur.extend_from_slice(&hotspot.1.to_le_bytes());
        cur.extend_from_slice(&(dib.len() as u32).to_le_bytes());
        cur.extend_from_slice(&22u32.to_le_bytes());
        cur.extend_from_slice(&dib);
        cur
    }

    #[test]
    fn decodes_cursor_with_hotspot_and_mask() {
        let c = decode(&cur_file((5, 9))).expect("decodes");
        assert_eq!(c.hotspot, (5, 9));
        let img = image::load_from_memory(&c.png).unwrap().to_rgba8();
        assert_eq!((img.width(), img.height()), (32, 32));
        assert_eq!(img.get_pixel(0, 0).0, [0, 0, 0, 255]);
        assert_eq!(img.get_pixel(31, 0).0[3], 0);
    }

    #[test]
    fn rejects_icons_and_garbage() {
        let mut ico = cur_file((0, 0));
        ico[2] = 1;
        assert!(decode(&ico).is_none());
        assert!(decode(b"RIFF....ACON").is_none());
        assert!(decode(&[0, 0, 2, 0, 1, 0]).is_none());
    }
}

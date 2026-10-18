//! Parsers for the text files of a classic skin.

use serde::Serialize;

/// The 24 visualizer colors from `viscolor.txt`:
/// 0 background, 1 dots, 2–17 spectrum bars (top to bottom), 18–22 oscilloscope, 23 peaks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VisColors(pub [[u8; 3]; 24]);

impl Default for VisColors {
    fn default() -> Self {
        let mut c = [[0u8; 3]; 24];
        c[0] = [0, 0, 0];
        c[1] = [40, 40, 52];
        for i in 0..16 {
            // Bars: warm at the top, green at the bottom.
            let t = i as f32 / 15.0;
            c[2 + i] = [(230.0 * (1.0 - t)) as u8 + 20, (120.0 + 110.0 * t) as u8, 60];
        }
        for (i, v) in [200u8, 180, 160, 140, 120].into_iter().enumerate() {
            c[18 + i] = [v, v, v];
        }
        c[23] = [220, 220, 230];
        Self(c)
    }
}

impl VisColors {
    /// Each line holds `r,g,b` optionally followed by a comment. Missing or broken
    /// lines keep the default color, like Winamp does.
    pub fn parse(text: &str) -> Self {
        let mut colors = Self::default();
        let mut slot = 0;
        for line in text.lines() {
            if slot >= 24 {
                break;
            }
            let nums: Vec<u8> = line
                .split(|c: char| !c.is_ascii_digit())
                .filter(|s| !s.is_empty())
                .take(3)
                .filter_map(|s| s.parse::<u32>().ok().map(|n| n.min(255) as u8))
                .collect();
            if let [r, g, b] = nums[..] {
                colors.0[slot] = [r, g, b];
                slot += 1;
            }
        }
        colors
    }
}

/// Playlist colors and font from the `[Text]` section of `pledit.txt`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PlaylistStyle {
    pub normal: String,
    pub current: String,
    pub normal_bg: String,
    pub selected_bg: String,
    pub font: String,
}

impl Default for PlaylistStyle {
    fn default() -> Self {
        Self {
            normal: "#00e05a".into(),
            current: "#ffffff".into(),
            normal_bg: "#000000".into(),
            selected_bg: "#1c3a6e".into(),
            font: "Arial".into(),
        }
    }
}

impl PlaylistStyle {
    pub fn parse(text: &str) -> Self {
        let mut style = Self::default();
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else { continue };
            let value = value.trim();
            let color = || normalize_color(value);
            match key.trim().to_ascii_lowercase().as_str() {
                "normal" => style.normal = color().unwrap_or(style.normal),
                "current" => style.current = color().unwrap_or(style.current),
                "normalbg" => style.normal_bg = color().unwrap_or(style.normal_bg),
                "selectedbg" => style.selected_bg = color().unwrap_or(style.selected_bg),
                "font" if !value.is_empty() => style.font = value.to_string(),
                _ => {}
            }
        }
        style
    }
}

/// Accept `#RRGGBB` or bare `RRGGBB`, returning lowercase `#rrggbb`.
fn normalize_color(value: &str) -> Option<String> {
    let hex = value.trim_start_matches('#');
    let hex = hex.get(..6)?;
    hex.chars().all(|c| c.is_ascii_hexdigit()).then(|| format!("#{}", hex.to_ascii_lowercase()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn viscolor_ignores_comments_and_clamps() {
        let v = VisColors::parse("0,0,0, // background\n 24 , 33 , 41 // dots\n300,1,2\n");
        assert_eq!(v.0[0], [0, 0, 0]);
        assert_eq!(v.0[1], [24, 33, 41]);
        assert_eq!(v.0[2], [255, 1, 2]);
        assert_eq!(v.0[3], VisColors::default().0[3]);
    }

    #[test]
    fn pledit_reads_text_section() {
        let p = PlaylistStyle::parse("[Text]\r\nNormal=#00FF00\r\nCurrent=FFFFFF\r\nNormalBG=#000000\r\nSelectedBG=bad\r\nFont=Tahoma\r\n");
        assert_eq!(p.normal, "#00ff00");
        assert_eq!(p.current, "#ffffff");
        assert_eq!(p.selected_bg, PlaylistStyle::default().selected_bg);
        assert_eq!(p.font, "Tahoma");
    }
}

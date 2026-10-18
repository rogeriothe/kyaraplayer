//! `region.txt`: polygons that give the main and equalizer windows a custom shape.
//!
//! ```text
//! [Normal]
//! NumPoints=4,4
//! PointList=0,0, 275,0, 275,14, 0,14,  0,14, 275,14, 275,116, 0,116
//! ```
//! Sections: `Normal`, `WindowShade`, `Equalizer`, `EqualizerWS`.

use std::collections::HashMap;

/// Polygons by lowercase section name; each polygon is a list of (x, y) points.
pub type Regions = HashMap<String, Vec<Vec<(i32, i32)>>>;

pub fn parse(text: &str) -> Regions {
    let mut raw: HashMap<String, (Vec<usize>, Vec<i32>)> = HashMap::new();
    let mut section: Option<String> = None;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with(';') || line.starts_with("//") || line.is_empty() {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            section = Some(name.trim().to_ascii_lowercase());
            continue;
        }
        let (Some(sec), Some((key, value))) = (&section, line.split_once('=')) else { continue };
        let numbers = value
            .split(|c: char| c == ',' || c.is_whitespace())
            .filter_map(|n| n.trim().parse::<i32>().ok());
        let entry = raw.entry(sec.clone()).or_default();
        match key.trim().to_ascii_lowercase().as_str() {
            "numpoints" => entry.0 = numbers.filter_map(|n| usize::try_from(n).ok()).collect(),
            "pointlist" => entry.1 = numbers.collect(),
            _ => {}
        }
    }

    raw.into_iter()
        .filter_map(|(name, (counts, coords))| {
            let points: Vec<(i32, i32)> = coords.chunks_exact(2).map(|p| (p[0], p[1])).collect();
            let mut polygons = Vec::new();
            let mut start = 0;
            for n in counts {
                let Some(poly) = points.get(start..start + n) else { break };
                if n >= 3 {
                    polygons.push(poly.to_vec());
                }
                start += n;
            }
            (!polygons.is_empty()).then_some((name, polygons))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_multiple_polygons_and_sections() {
        let r = parse(
            "; comment\n[Normal]\nNumPoints=4, 3\nPointList=0,0 275,0 275,14 0,14, 0,20,10,20,0,30\n\
             [WindowShade]\nNumPoints=4\nPointList=0,0,275,0,275,14,0,14\n[Empty]\nNumPoints=0\n",
        );
        assert_eq!(r["normal"].len(), 2);
        assert_eq!(r["normal"][1], vec![(0, 20), (10, 20), (0, 30)]);
        assert_eq!(r["windowshade"][0][2], (275, 14));
        assert!(!r.contains_key("empty"));
    }

    #[test]
    fn truncated_point_lists_keep_complete_polygons() {
        let r = parse("[Equalizer]\nNumPoints=3,4\nPointList=0,0,5,0,5,5,1,1\n");
        assert_eq!(r["equalizer"].len(), 1);
    }
}

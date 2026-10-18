//! Decode a skin and write its sheets as PNG files.
//!
//! cargo run -p skin --example dump -- <skin.wsz|dir> <out-dir>

fn main() {
    let mut args = std::env::args().skip(1);
    let (Some(src), Some(out)) = (args.next(), args.next()) else {
        eprintln!("usage: dump <skin.wsz|dir> <out-dir>");
        std::process::exit(2);
    };
    let skin = skin::Skin::load(src.as_ref()).unwrap_or_else(|e| panic!("{src}: {e}"));
    std::fs::create_dir_all(&out).expect("create out dir");
    for (name, png) in &skin.sheets {
        std::fs::write(format!("{out}/{name}.png"), png).expect("write sheet");
    }
    for (name, cursor) in &skin.cursors {
        std::fs::write(format!("{out}/cursor-{name}.png"), &cursor.png).expect("write cursor");
    }
    println!("{}: {} sheets, viscolor[0..3] = {:?}", skin.name, skin.sheets.len(), &skin.viscolors.0[..3]);
    let mut cursors: Vec<_> = skin.cursors.iter().map(|(k, c)| (k, c.hotspot)).collect();
    cursors.sort();
    println!("cursors: {cursors:?}");
    println!("regions: {:?}", skin.regions);
}

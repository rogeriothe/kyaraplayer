//! Window layout: visibility, docking, snapping, windowshade and double size.
//!
//! Like the original, windows are dragged by the app (not the OS) so a docked group
//! moves together and edges snap within 10 px. Sizes come from the known design
//! sizes rather than the OS, whose answers are unreliable before a window is mapped.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, WebviewWindow};

use crate::config::{self, Config};

pub const LABELS: [&str; 3] = ["main", "equalizer", "playlist"];
const SNAP: f64 = 10.0;
/// Edges closer than this count as touching (rounding of scaled positions).
const TOUCH: f64 = 1.5;
const W: f64 = 275.0;
const H: f64 = 116.0;
const SHADE_H: f64 = 14.0;

#[derive(Serialize, Clone)]
pub struct WindowsView {
    equalizer: bool,
    playlist: bool,
    main_shade: bool,
    eq_shade: bool,
    pl_shade: bool,
    double: bool,
    always_on_top: bool,
}

fn view(app: &AppHandle) -> WindowsView {
    let cfg = config::load(app);
    let visible = |label: &str| app.get_webview_window(label).and_then(|w| w.is_visible().ok()).unwrap_or(false);
    WindowsView {
        equalizer: visible("equalizer"),
        playlist: visible("playlist"),
        main_shade: cfg.main_shade,
        eq_shade: cfg.eq_shade,
        pl_shade: cfg.pl_shade,
        double: cfg.double,
        always_on_top: cfg.always_on_top,
    }
}

fn broadcast(app: &AppHandle) -> WindowsView {
    let v = view(app);
    let _ = app.emit("windows", &v);
    v
}

// ---------------------------------------------------------------- geometry

#[derive(Debug, Clone, Copy, PartialEq)]
struct Rect {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

impl Rect {
    fn right(&self) -> f64 {
        self.x + self.w
    }
    fn bottom(&self) -> f64 {
        self.y + self.h
    }
    fn overlaps_x(&self, o: &Rect, margin: f64) -> bool {
        self.x < o.right() + margin && o.x < self.right() + margin
    }
    fn overlaps_y(&self, o: &Rect, margin: f64) -> bool {
        self.y < o.bottom() + margin && o.y < self.bottom() + margin
    }
    /// Edges touch along one axis while overlapping along the other.
    fn docked_to(&self, o: &Rect) -> bool {
        let vertical = ((self.bottom() - o.y).abs() < TOUCH || (o.bottom() - self.y).abs() < TOUCH)
            && self.overlaps_x(o, 0.0);
        let horizontal = ((self.right() - o.x).abs() < TOUCH || (o.right() - self.x).abs() < TOUCH)
            && self.overlaps_y(o, 0.0);
        vertical || horizontal
    }
}

/// Design size of a window for the given settings.
fn size_of(label: &str, cfg: &Config) -> (f64, f64) {
    let k = if cfg.double { 2.0 } else { 1.0 };
    match label {
        "main" => (W * k, if cfg.main_shade { SHADE_H } else { H } * k),
        "equalizer" => (W * k, if cfg.eq_shade { SHADE_H } else { H } * k),
        _ => {
            let (w, h) = cfg.playlist_size;
            (w.max(275) as f64, if cfg.pl_shade { SHADE_H } else { h.max(116) as f64 })
        }
    }
}

/// Content position. Undecorated windows have no frame, but some window managers
/// (e.g. Muffin) still report frame extents, which skews `outer_position`; the inner
/// position matches what `set_position` places.
fn position(win: &WebviewWindow) -> Option<(f64, f64)> {
    let scale = win.scale_factor().ok()?;
    let p = win.inner_position().ok()?.to_logical::<f64>(scale);
    Some((p.x, p.y))
}

/// Rects of the visible windows.
fn rects(app: &AppHandle, cfg: &Config) -> Vec<(String, Rect)> {
    LABELS
        .iter()
        .filter_map(|&label| {
            let win = app.get_webview_window(label)?;
            if !win.is_visible().unwrap_or(false) {
                return None;
            }
            let (x, y) = position(&win)?;
            let (w, h) = size_of(label, cfg);
            Some((label.to_string(), Rect { x, y, w, h }))
        })
        .collect()
}

/// Windows connected to `start` through touching edges (including `start`).
fn docked_group(rects: &[(String, Rect)], start: &str) -> Vec<String> {
    let mut group = vec![start.to_string()];
    let mut queue = VecDeque::from([start.to_string()]);
    while let Some(cur) = queue.pop_front() {
        let Some((_, r)) = rects.iter().find(|(l, _)| *l == cur) else { continue };
        for (label, other) in rects {
            if !group.contains(label) && r.docked_to(other) {
                group.push(label.clone());
                queue.push_back(label.clone());
            }
        }
    }
    group
}

fn set_pos(app: &AppHandle, label: &str, x: f64, y: f64) {
    if let Some(win) = app.get_webview_window(label) {
        let _ = win.set_position(LogicalPosition::new(x.round(), y.round()));
    }
}

/// Apply the design size, pinning min/max so the OS cannot resize fixed windows.
fn apply_size(app: &AppHandle, label: &str, cfg: &Config) {
    let Some(win) = app.get_webview_window(label) else { return };
    let (w, h) = size_of(label, cfg);
    let size = LogicalSize::new(w, h);
    let (min, max) = if label == "playlist" && !cfg.pl_shade {
        (LogicalSize::new(275.0, 116.0), None)
    } else {
        (size, Some(size))
    };
    // Loosen first so the new size is never outside the old limits.
    let _ = win.set_min_size(Some(LogicalSize::new(1.0, 1.0)));
    let _ = win.set_max_size(None::<LogicalSize<f64>>);
    let _ = win.set_size(size);
    let _ = win.set_min_size(Some(min));
    let _ = win.set_max_size(max);
}

/// Change settings that alter `labels`' sizes, keeping docked neighbours attached:
/// windows below or to the right of a resized window shift by its size change.
fn resize(app: &AppHandle, labels: &[&str], change: impl FnOnce(&mut Config)) {
    let before = config::load(app);
    let rects_before = rects(app, &before);
    config::update(app, change);
    let after = config::load(app);

    let mut offsets: HashMap<String, (f64, f64)> = HashMap::new();
    for &label in labels {
        let Some((_, old)) = rects_before.iter().find(|(l, _)| l == label) else {
            apply_size(app, label, &after);
            continue;
        };
        let (w, h) = size_of(label, &after);
        let (dw, dh) = (w - old.w, h - old.h);
        for other in docked_group(&rects_before, label) {
            if other == label {
                continue;
            }
            let r = rects_before.iter().find(|(l, _)| *l == other).map(|(_, r)| *r).unwrap();
            let entry = offsets.entry(other).or_default();
            if r.y >= old.bottom() - TOUCH {
                entry.1 += dh;
            }
            if r.x >= old.right() - TOUCH {
                entry.0 += dw;
            }
        }
    }
    for &label in labels {
        apply_size(app, label, &after);
    }
    for (label, (dx, dy)) in offsets {
        if let Some((_, r)) = rects_before.iter().find(|(l, _)| *l == label) {
            set_pos(app, &label, r.x + dx, r.y + dy);
        }
    }
    save_positions(app);
    broadcast(app);
}

fn save_positions(app: &AppHandle) {
    let positions: HashMap<String, (f64, f64)> = LABELS
        .iter()
        .filter_map(|&l| app.get_webview_window(l).and_then(|w| position(&w)).map(|p| (l.to_string(), p)))
        .collect();
    config::update(app, |c| c.positions = positions);
}

// ---------------------------------------------------------------- dragging

struct Drag {
    /// Cursor (logical px) when the drag started.
    origin: (f64, f64),
    moving: Vec<(String, Rect)>,
    fixed: Vec<Rect>,
    area: Option<Rect>,
}

#[derive(Default)]
pub struct DragState(Mutex<Option<Drag>>);

/// Cursor position in logical px. Read from the OS: the webview's screen
/// coordinates lag behind while its own window is being moved.
fn cursor(app: &AppHandle, win: &WebviewWindow) -> Option<(f64, f64)> {
    let scale = win.scale_factor().ok()?;
    let p = app.cursor_position().ok()?.to_logical::<f64>(scale);
    Some((p.x, p.y))
}

fn work_area(win: &WebviewWindow) -> Option<Rect> {
    let m = win.current_monitor().ok().flatten()?;
    let scale = m.scale_factor();
    let pos = m.work_area().position.to_logical::<f64>(scale);
    let size = m.work_area().size.to_logical::<f64>(scale);
    Some(Rect { x: pos.x, y: pos.y, w: size.width, h: size.height })
}

/// Wayland hides the global cursor position and forbids placing windows, so
/// there the page falls back to the OS drag (no docking).
fn can_manage_drag() -> bool {
    let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some()
        && std::env::var("GDK_BACKEND").map_or(true, |b| !b.starts_with("x11"));
    !(cfg!(target_os = "linux") && wayland)
}

/// Start dragging `label`. The main window takes its docked group along; the
/// others move alone (and undock), as in the original. Returns false when the
/// page must use the OS drag instead.
#[tauri::command]
pub fn drag_start(label: String, app: AppHandle, state: tauri::State<'_, DragState>) -> bool {
    if !can_manage_drag() {
        return false;
    }
    let cfg = config::load(&app);
    let all = rects(&app, &cfg);
    let group = if label == "main" { docked_group(&all, "main") } else { vec![label.clone()] };
    let (moving, fixed): (Vec<_>, Vec<_>) = all.into_iter().partition(|(l, _)| group.contains(l));
    let Some(win) = app.get_webview_window(&label) else { return false };
    let Some(origin) = cursor(&app, &win) else { return false };
    let area = work_area(&win);
    *state.0.lock().unwrap() = Some(Drag { origin, moving, fixed: fixed.into_iter().map(|(_, r)| r).collect(), area });
    true
}

/// Smallest adjustment within the snap distance, if any.
fn best(candidates: impl Iterator<Item = f64>) -> f64 {
    candidates.filter(|d| d.abs() <= SNAP).min_by(|a, b| a.abs().total_cmp(&b.abs())).unwrap_or(0.0)
}

/// Move the dragged windows to follow the cursor since the drag started.
#[tauri::command]
pub fn drag_move(app: AppHandle, state: tauri::State<'_, DragState>) {
    let guard = state.0.lock().unwrap();
    let Some(drag) = guard.as_ref() else { return };
    let Some(win) = drag.moving.first().and_then(|(l, _)| app.get_webview_window(l)) else { return };
    let Some((cx, cy)) = cursor(&app, &win) else { return };
    let (dx, dy) = (cx - drag.origin.0, cy - drag.origin.1);
    let moved: Vec<Rect> = drag.moving.iter().map(|(_, r)| Rect { x: r.x + dx, y: r.y + dy, ..*r }).collect();

    let mut xs = Vec::new();
    let mut ys = Vec::new();
    for m in &moved {
        for s in &drag.fixed {
            if m.overlaps_y(s, SNAP) {
                xs.extend([s.x - m.x, s.right() - m.x, s.x - m.right(), s.right() - m.right()]);
            }
            if m.overlaps_x(s, SNAP) {
                ys.extend([s.y - m.y, s.bottom() - m.y, s.y - m.bottom(), s.bottom() - m.bottom()]);
            }
        }
        if let Some(a) = &drag.area {
            xs.extend([a.x - m.x, a.right() - m.right()]);
            ys.extend([a.y - m.y, a.bottom() - m.bottom()]);
        }
    }
    let (sx, sy) = (best(xs.into_iter()), best(ys.into_iter()));
    for ((label, _), r) in drag.moving.iter().zip(&moved) {
        set_pos(&app, label, r.x + sx, r.y + sy);
    }
}

#[tauri::command]
pub fn drag_end(app: AppHandle, state: tauri::State<'_, DragState>) {
    *state.0.lock().unwrap() = None;
    save_positions(&app);
}

// ---------------------------------------------------------------- commands

#[tauri::command]
pub fn windows_view(app: AppHandle) -> WindowsView {
    view(&app)
}

/// Show or hide a secondary window (`"equalizer"`, `"playlist"`), remembering the choice.
#[tauri::command]
pub fn set_window_visible(label: String, visible: bool, app: AppHandle) -> Result<WindowsView, String> {
    let win = app.get_webview_window(&label).ok_or_else(|| format!("no window {label}"))?;
    if visible {
        win.show().map_err(|e| e.to_string())?;
    } else {
        win.hide().map_err(|e| e.to_string())?;
    }
    match label.as_str() {
        "playlist" => config::update(&app, |c| c.playlist_open = visible),
        "equalizer" => config::update(&app, |c| c.eq_open = visible),
        _ => {}
    }
    Ok(broadcast(&app))
}

/// Toggle windowshade (compact 14 px) mode of one window.
#[tauri::command]
pub fn set_shade(label: String, on: bool, app: AppHandle) -> WindowsView {
    resize(&app, &[label.as_str()], |c| match label.as_str() {
        "main" => c.main_shade = on,
        "equalizer" => c.eq_shade = on,
        _ => c.pl_shade = on,
    });
    view(&app)
}

/// Double size for the main and equalizer windows.
#[tauri::command]
pub fn set_double(on: bool, app: AppHandle) -> WindowsView {
    resize(&app, &["main", "equalizer"], |c| c.double = on);
    view(&app)
}

/// Open the file info window for `path`, replacing one already open.
#[tauri::command]
pub fn open_file_info(path: String, app: AppHandle) -> Result<(), String> {
    if let Some(w) = app.get_webview_window("fileinfo") {
        let _ = w.destroy();
    }
    let encoded: String = path
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect();
    let url = tauri::WebviewUrl::App(format!("fileinfo.html?path={encoded}").into());
    let mut builder = tauri::WebviewWindowBuilder::new(&app, "fileinfo", url)
        .title("Informações do arquivo")
        .inner_size(480.0, 400.0)
        .min_inner_size(400.0, 360.0)
        .maximizable(false)
        .minimizable(false)
        .skip_taskbar(true)
        .center();
    if let Some(main) = app.get_webview_window("main") {
        builder = builder.parent(&main).map_err(|e| e.to_string())?;
    }
    builder.build().map(|_| ()).map_err(|e| e.to_string())
}

/// Open (or focus) the "Open URL" dialog.
#[tauri::command]
pub fn open_url_dialog(app: AppHandle) -> Result<(), String> {
    if let Some(w) = app.get_webview_window("openurl") {
        return w.set_focus().map_err(|e| e.to_string());
    }
    let mut builder = tauri::WebviewWindowBuilder::new(&app, "openurl", tauri::WebviewUrl::App("openurl.html".into()))
        .title("Abrir endereço")
        .inner_size(460.0, 130.0)
        .resizable(false)
        .maximizable(false)
        .minimizable(false)
        .skip_taskbar(true)
        .center();
    if let Some(main) = app.get_webview_window("main") {
        builder = builder.parent(&main).map_err(|e| e.to_string())?;
    }
    builder.build().map(|_| ()).map_err(|e| e.to_string())
}

/// Keep every player window above other windows.
#[tauri::command]
pub fn set_always_on_top(on: bool, app: AppHandle) -> WindowsView {
    apply_always_on_top(&app, on);
    config::update(&app, |c| c.always_on_top = on);
    broadcast(&app)
}

fn apply_always_on_top(app: &AppHandle, on: bool) {
    for label in LABELS {
        if let Some(w) = app.get_webview_window(label) {
            if let Err(e) = w.set_always_on_top(on) {
                log::warn!("always on top for {label}: {e}");
            }
        }
    }
}

/// Remember the playlist size after the user resizes it.
#[tauri::command]
pub fn save_playlist_size(width: u32, height: u32, app: AppHandle) {
    config::update(&app, |c| c.playlist_size = (width, height));
}

// ---------------------------------------------------------------- startup

/// Restore sizes and positions; default to the classic stack centered on screen.
pub fn setup(app: &AppHandle) {
    let cfg = config::load(app);
    for label in LABELS {
        apply_size(app, label, &cfg);
    }
    let Some(main) = app.get_webview_window("main") else { return };

    let all_saved = LABELS.iter().all(|l| cfg.positions.contains_key(*l));
    if all_saved {
        for (label, (x, y)) in &cfg.positions {
            set_pos(app, label, *x, *y);
        }
    } else if let Some(area) = work_area(&main) {
        let (_, main_h) = size_of("main", &cfg);
        let (_, eq_h) = size_of("equalizer", &cfg);
        let (_, pl_h) = size_of("playlist", &cfg);
        let x = area.x + (area.w - W) / 2.0;
        let y = area.y + ((area.h - (main_h + eq_h + pl_h)) / 2.0).max(0.0);
        set_pos(app, "main", x, y);
        set_pos(app, "equalizer", x, y + main_h);
        set_pos(app, "playlist", x, y + main_h + eq_h);
    }

    let show = |label: &str| {
        if let Some(w) = app.get_webview_window(label) {
            let _ = w.show();
        }
    };
    if cfg.eq_open {
        show("equalizer");
    }
    if cfg.playlist_open {
        show("playlist");
    }
    if cfg.always_on_top {
        apply_always_on_top(app, true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(x: f64, y: f64, w: f64, h: f64) -> Rect {
        Rect { x, y, w, h }
    }

    #[test]
    fn docking_needs_touching_edges_and_overlap() {
        let main = r(0.0, 0.0, 275.0, 116.0);
        assert!(main.docked_to(&r(0.0, 116.0, 275.0, 116.0))); // below
        assert!(main.docked_to(&r(275.0, 50.0, 275.0, 116.0))); // right
        assert!(!main.docked_to(&r(0.0, 120.0, 275.0, 116.0))); // gap
        assert!(!main.docked_to(&r(275.0, 116.0, 275.0, 116.0))); // corner only
    }

    #[test]
    fn group_follows_chains() {
        let rects = vec![
            ("main".to_string(), r(0.0, 0.0, 275.0, 116.0)),
            ("equalizer".to_string(), r(0.0, 116.0, 275.0, 116.0)),
            ("playlist".to_string(), r(0.0, 232.0, 275.0, 232.0)),
        ];
        assert_eq!(docked_group(&rects, "main").len(), 3);
        let mut apart = rects.clone();
        apart[1].1.y = 300.0;
        apart[2].1.y = 600.0;
        assert_eq!(docked_group(&apart, "main"), vec!["main".to_string()]);
    }

    #[test]
    fn snap_picks_smallest_within_range() {
        assert_eq!(best([12.0, -4.0, 7.0].into_iter()), -4.0);
        assert_eq!(best([11.0, -20.0].into_iter()), 0.0);
    }
}

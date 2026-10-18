//! Equalizer commands: settings, presets and preset files.

use std::path::{Path, PathBuf};

use audio_engine::eqf::{self, Preset};
use audio_engine::EqSettings;
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use crate::config;
use crate::player::Player;

#[derive(Serialize, Clone)]
pub struct EqView {
    settings: EqSettings,
    auto: bool,
    builtin: Vec<Preset>,
    user: Vec<Preset>,
    auto_presets: Vec<Preset>,
    /// Key of the playing track for auto presets (its file name), if any.
    current_file: Option<String>,
}

fn view(app: &AppHandle, player: &Player) -> EqView {
    let cfg = config::load(app);
    EqView {
        settings: player.engine.eq(),
        auto: cfg.eq_auto,
        builtin: eqf::builtin(),
        user: cfg.eq_presets,
        auto_presets: cfg.eq_auto_presets,
        current_file: current_file(player),
    }
}

/// Auto presets are keyed by file name (the whole address for streams), like the original.
fn auto_key(path: &Path) -> Option<String> {
    let s = path.to_str()?;
    if audio_engine::stream::is_url(s) {
        return Some(s.to_string());
    }
    path.file_name().map(|n| n.to_string_lossy().into_owned())
}

fn current_file(player: &Player) -> Option<String> {
    player.engine.status().track.and_then(|t| auto_key(&t.path))
}

/// With "Auto" on, load the preset saved for the track that just started.
pub fn on_track_started(app: &AppHandle, player: &Player, path: &Path) {
    let cfg = config::load(app);
    let key = auto_key(path);
    let preset = key.and_then(|k| cfg.eq_auto_presets.iter().find(|p| p.name.eq_ignore_ascii_case(&k)).cloned());
    match preset {
        Some(p) if cfg.eq_auto => {
            apply(app, player, &p);
        }
        // Still tell the window which file is playing (for "save auto preset").
        _ => {
            broadcast(app, player);
        }
    }
}

fn apply(app: &AppHandle, player: &Player, preset: &Preset) -> EqView {
    let mut s = player.engine.eq();
    s.preamp_db = preset.preamp_db;
    s.bands_db = preset.bands_db;
    player.engine.set_eq(&s);
    config::update(app, |c| c.eq = s);
    broadcast(app, player)
}

fn broadcast(app: &AppHandle, player: &Player) -> EqView {
    let v = view(app, player);
    let _ = app.emit("eq", &v);
    v
}

/// Apply the saved settings to the engine at startup.
pub fn restore(app: &AppHandle, player: &Player) {
    player.engine.set_eq(&config::load(app).eq);
}

#[tauri::command]
pub fn eq_view(player: State<'_, Player>, app: AppHandle) -> EqView {
    view(&app, &player)
}

/// Apply settings. `persist` is false while a slider is being dragged.
#[tauri::command]
pub fn eq_set(settings: EqSettings, persist: bool, player: State<'_, Player>, app: AppHandle) {
    player.engine.set_eq(&settings);
    if persist {
        config::update(&app, |c| c.eq = settings);
    }
}

#[tauri::command]
pub fn eq_set_auto(on: bool, player: State<'_, Player>, app: AppHandle) -> EqView {
    config::update(&app, |c| c.eq_auto = on);
    broadcast(&app, &player)
}

/// Load a preset's gains, keeping the on/off state and band layout.
#[tauri::command]
pub fn eq_apply_preset(preset: Preset, player: State<'_, Player>, app: AppHandle) -> EqView {
    apply(&app, &player, &preset)
}

/// Save the current gains as the auto preset of the playing track.
#[tauri::command]
pub fn eq_save_auto_preset(player: State<'_, Player>, app: AppHandle) -> Result<EqView, String> {
    let name = current_file(&player).ok_or("nothing is playing")?;
    let s = player.engine.eq();
    let preset = Preset { name: name.clone(), preamp_db: s.preamp_db, bands_db: s.bands_db };
    config::update(&app, |c| {
        c.eq_auto_presets.retain(|p| !p.name.eq_ignore_ascii_case(&name));
        c.eq_auto_presets.push(preset);
    });
    Ok(broadcast(&app, &player))
}

#[tauri::command]
pub fn eq_delete_auto_preset(name: String, player: State<'_, Player>, app: AppHandle) -> EqView {
    config::update(&app, |c| c.eq_auto_presets.retain(|p| p.name != name));
    broadcast(&app, &player)
}

/// Save the current gains as a user preset, replacing one with the same name.
#[tauri::command]
pub fn eq_save_preset(name: String, player: State<'_, Player>, app: AppHandle) -> EqView {
    let s = player.engine.eq();
    let preset = Preset { name: name.clone(), preamp_db: s.preamp_db, bands_db: s.bands_db };
    config::update(&app, |c| {
        c.eq_presets.retain(|p| p.name != name);
        c.eq_presets.push(preset);
    });
    broadcast(&app, &player)
}

#[tauri::command]
pub fn eq_delete_preset(name: String, player: State<'_, Player>, app: AppHandle) -> EqView {
    config::update(&app, |c| c.eq_presets.retain(|p| p.name != name));
    broadcast(&app, &player)
}

/// Import an `.eqf`/`.q1` file: its presets join the user list and the first is applied.
#[tauri::command]
pub fn eq_import(path: PathBuf, player: State<'_, Player>, app: AppHandle) -> Result<EqView, String> {
    let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
    let presets = eqf::parse(&bytes).ok_or("not an equalizer preset file")?;
    let first = presets.first().cloned().ok_or("file has no presets")?;
    config::update(&app, |c| {
        for p in presets {
            c.eq_presets.retain(|x| x.name != p.name);
            c.eq_presets.push(p);
        }
    });
    Ok(eq_apply_preset(first, player, app))
}

/// Export the current gains as a single-preset `.eqf` named after the file.
#[tauri::command]
pub fn eq_export(path: PathBuf, player: State<'_, Player>) -> Result<(), String> {
    let s = player.engine.eq();
    let name = path.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "Preset".into());
    let bytes = eqf::serialize(&[Preset { name, preamp_db: s.preamp_db, bands_db: s.bands_db }]);
    std::fs::write(&path, bytes).map_err(|e| e.to_string())
}

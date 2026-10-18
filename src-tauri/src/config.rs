//! Persistent user settings (`config.json` in the app config directory).

use std::collections::HashMap;
use std::path::PathBuf;

use audio_engine::eqf::Preset;
use audio_engine::EqSettings;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Last loaded skin; `None` means the built-in skin.
    pub skin: Option<PathBuf>,
    pub volume: f32,
    pub balance: f32,
    pub shuffle: bool,
    pub repeat: bool,
    pub playlist_open: bool,
    /// Playlist window size in logical pixels.
    pub playlist_size: (u32, u32),
    /// Index of the current playlist entry when the app closed.
    pub playlist_current: Option<usize>,
    pub eq: EqSettings,
    pub eq_open: bool,
    pub eq_auto: bool,
    /// User presets (built-in ones are not stored).
    pub eq_presets: Vec<Preset>,
    /// Presets loaded automatically ("Auto" button), named by the file they belong to.
    pub eq_auto_presets: Vec<Preset>,
    /// Window positions (logical px) by label.
    pub positions: HashMap<String, (f64, f64)>,
    pub main_shade: bool,
    pub eq_shade: bool,
    pub pl_shade: bool,
    /// Double size for the main and equalizer windows.
    pub double: bool,
    pub always_on_top: bool,
    /// Ctrl+Alt+... hotkeys that work while other programs have focus.
    pub global_hotkeys: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            skin: None,
            volume: 1.0,
            balance: 0.0,
            shuffle: false,
            repeat: false,
            playlist_open: true,
            playlist_size: (275, 232),
            playlist_current: None,
            eq: EqSettings::default(),
            eq_open: true,
            eq_auto: false,
            eq_presets: Vec::new(),
            eq_auto_presets: Vec::new(),
            positions: HashMap::new(),
            main_shade: false,
            eq_shade: false,
            pl_shade: false,
            double: false,
            always_on_top: false,
            global_hotkeys: true,
        }
    }
}

pub fn dir(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_config_dir().ok()
}

fn path(app: &AppHandle) -> Option<PathBuf> {
    dir(app).map(|d| d.join("config.json"))
}

pub fn load(app: &AppHandle) -> Config {
    let Some(path) = path(app) else { return Config::default() };
    match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|e| {
            log::warn!("ignoring invalid {}: {e}", path.display());
            Config::default()
        }),
        Err(_) => Config::default(),
    }
}

pub fn save(app: &AppHandle, config: &Config) {
    let Some(path) = path(app) else { return };
    let result = path
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| std::fs::write(&path, serde_json::to_vec_pretty(config).unwrap_or_default()));
    if let Err(e) = result {
        log::warn!("cannot save {}: {e}", path.display());
    }
}

/// Serializes read-modify-write cycles from different threads.
static UPDATE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Load, change and save in one step.
pub fn update(app: &AppHandle, f: impl FnOnce(&mut Config)) {
    let _guard = UPDATE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut cfg = load(app);
    f(&mut cfg);
    save(app, &cfg);
}

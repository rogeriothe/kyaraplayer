//! System-wide hotkeys with the classic Ctrl+Alt bindings. Each one is forwarded to the
//! main window as a `hotkey` event naming the action, so it behaves like the local key.
//! Wayland compositors do not allow global grabs; registration then fails quietly.

use tauri::{AppHandle, Emitter};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

use crate::config;

const HOTKEYS: &[(&str, &str)] = &[
    ("Ctrl+Alt+Home", "play"),
    ("Ctrl+Alt+Insert", "pause"),
    ("Ctrl+Alt+End", "stop"),
    ("Ctrl+Alt+PageUp", "prev"),
    ("Ctrl+Alt+PageDown", "next"),
    ("Ctrl+Alt+Up", "volume-up"),
    ("Ctrl+Alt+Down", "volume-down"),
    ("Ctrl+Alt+Left", "back"),
    ("Ctrl+Alt+Right", "forward"),
];

pub fn start(app: &AppHandle) {
    if config::load(app).global_hotkeys {
        register(app);
    }
}

fn register(app: &AppHandle) {
    let shortcuts = app.global_shortcut();
    for &(keys, action) in HOTKEYS {
        let result = shortcuts.on_shortcut(keys, move |app, _, event| {
            if event.state == ShortcutState::Pressed {
                let _ = app.emit_to("main", "hotkey", action);
            }
        });
        // Usually another program already owns the combination.
        if let Err(e) = result {
            log::warn!("global hotkey {keys}: {e}");
        }
    }
}

#[tauri::command]
pub fn set_global_hotkeys(on: bool, app: AppHandle) -> bool {
    let shortcuts = app.global_shortcut();
    if let Err(e) = shortcuts.unregister_all() {
        log::warn!("unregister global hotkeys: {e}");
    }
    if on {
        register(&app);
    }
    config::update(&app, |c| c.global_hotkeys = on);
    on
}

#[tauri::command]
pub fn global_hotkeys(app: AppHandle) -> bool {
    config::load(&app).global_hotkeys
}

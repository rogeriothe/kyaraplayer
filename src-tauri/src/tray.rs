//! Notification-area icon with playback controls.

use std::sync::Mutex;

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager};

use crate::config;
use crate::player::{self, Player};

const TRAY_ID: &str = "main";

pub fn start(app: &AppHandle) -> tauri::Result<()> {
    let item = |id: &str, text: &str| MenuItem::with_id(app, id, text, true, None::<&str>);
    let menu = Menu::with_items(
        app,
        &[
            &item("toggle-windows", "Mostrar/ocultar Kyara Player")?,
            &PredefinedMenuItem::separator(app)?,
            &item("prev", "Anterior")?,
            &item("play", "Play")?,
            &item("pause", "Pause")?,
            &item("stop", "Stop")?,
            &item("next", "Próxima")?,
            &PredefinedMenuItem::separator(app)?,
            &item("exit", "Sair")?,
        ],
    )?;
    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("Kyara Player")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| on_menu(app, event.id.as_ref()))
        // Not delivered on Linux (AppIndicator only offers the menu).
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } =
                event
            {
                toggle_windows(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    Ok(())
}

fn on_menu(app: &AppHandle, id: &str) {
    let player = app.state::<Player>();
    match id {
        "toggle-windows" => toggle_windows(app),
        "prev" => player::pl_prev(player, app.clone()),
        "play" => player::play_pressed(player, app.clone()),
        "pause" => player.engine.toggle_pause(),
        "stop" => player.engine.stop(),
        "next" => player::pl_next(player, app.clone()),
        "exit" => app.exit(0),
        _ => {}
    }
}

/// Hide every player window, or bring them back as they were.
fn toggle_windows(app: &AppHandle) {
    let Some(main) = app.get_webview_window("main") else { return };
    let visible = main.is_visible().unwrap_or(true) && !main.is_minimized().unwrap_or(false);
    if visible {
        for w in app.webview_windows().values() {
            let _ = w.hide();
        }
        return;
    }
    let cfg = config::load(app);
    let _ = main.unminimize();
    let _ = main.show();
    for (label, open) in [("equalizer", cfg.eq_open), ("playlist", cfg.playlist_open)] {
        if let (true, Some(w)) = (open, app.get_webview_window(label)) {
            let _ = w.show();
        }
    }
    let _ = main.set_focus();
}

/// Show the current track in the icon's tooltip.
pub fn set_title(app: &AppHandle, title: Option<&str>) {
    static SHOWN: Mutex<Option<String>> = Mutex::new(None);
    let Some(tray) = app.tray_by_id(TRAY_ID) else { return };
    let mut shown = SHOWN.lock().unwrap();
    if shown.as_deref() != title {
        *shown = title.map(str::to_string);
        let text = title.map_or_else(|| "Kyara Player".to_string(), |t| format!("{t} - Kyara Player"));
        let _ = tray.set_tooltip(Some(text));
    }
}

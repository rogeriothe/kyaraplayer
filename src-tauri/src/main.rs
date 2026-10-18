// Hide the console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod config;
mod eq;
mod hotkeys;
mod media;
mod player;
mod skins;
mod tray;
mod windows;

use std::path::{Path, PathBuf};

use tauri::{Manager, WindowEvent};

use player::Player;
use skins::SkinState;
use windows::DragState;

fn main() {
    // Warnings by default; RUST_LOG overrides.
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    let (player, meta_rx) = Player::new();
    let mut meta_rx = Some(meta_rx);

    tauri::Builder::default()
        // Must come first: a second launch hands its files to the running player and exits.
        .plugin(tauri_plugin_single_instance::init(|app, argv, cwd| {
            if let Some(main) = app.get_webview_window("main") {
                let _ = main.unminimize();
                let _ = main.show();
                let _ = main.set_focus();
            }
            open_paths(app, file_args(argv.into_iter().skip(1), Path::new(&cwd)));
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .manage(player)
        .manage(SkinState::default())
        .manage(DragState::default())
        .manage(media::Media::default())
        .register_uri_scheme_protocol("skin", |ctx, request| {
            skins::protocol(ctx.app_handle(), request.uri().path())
        })
        .setup(move |app| {
            let handle = app.handle();
            skins::restore(handle);
            player::start(handle, meta_rx.take().expect("setup runs once"));
            eq::restore(handle, &app.state::<Player>());
            windows::setup(handle);
            media::start(handle);
            hotkeys::start(handle);
            if let Err(e) = tray::start(handle) {
                log::warn!("no tray icon: {e}");
            }

            let cwd = std::env::current_dir().unwrap_or_default();
            open_paths(handle, file_args(std::env::args_os().skip(1), &cwd));
            Ok(())
        })
        .on_window_event(|window, event| {
            // Hidden secondary windows must not keep the app alive.
            if window.label() == "main" && matches!(event, WindowEvent::Destroyed) {
                window.app_handle().exit(0);
            }
        })
        .invoke_handler(tauri::generate_handler![
            player::toggle_pause,
            player::stop,
            player::seek,
            player::set_volume,
            player::set_balance,
            player::status,
            player::vis_frame,
            player::pl_view,
            player::pl_add,
            player::pl_open,
            player::pl_add_url,
            player::file_info,
            player::pl_file_info,
            player::file_info_save,
            windows::open_file_info,
            player::pl_remove,
            player::pl_crop,
            player::pl_clear,
            player::pl_remove_misc,
            player::pl_move,
            player::pl_sort,
            player::pl_play,
            player::pl_next,
            player::pl_prev,
            player::pl_jump,
            player::pl_set_shuffle,
            player::pl_set_repeat,
            player::pl_save,
            player::pl_load,
            eq::eq_view,
            eq::eq_set,
            eq::eq_set_auto,
            eq::eq_apply_preset,
            eq::eq_save_preset,
            eq::eq_delete_preset,
            eq::eq_save_auto_preset,
            eq::eq_delete_auto_preset,
            eq::eq_import,
            eq::eq_export,
            skins::skin_info,
            skins::load_skin,
            windows::windows_view,
            windows::set_window_visible,
            windows::save_playlist_size,
            windows::drag_start,
            windows::drag_move,
            windows::drag_end,
            windows::set_shade,
            windows::set_double,
            windows::set_always_on_top,
            windows::open_url_dialog,
            hotkeys::global_hotkeys,
            hotkeys::set_global_hotkeys,
        ])
        .build(tauri::generate_context!())
        .expect("error while building Kyara Player")
        .run(|_app, _event| {
            // macOS delivers files opened from Finder as an event, not as arguments.
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Opened { urls } = _event {
                let paths = urls.into_iter().filter_map(|u| u.to_file_path().ok()).collect();
                open_paths(_app, paths);
            }
        });
}

/// Files from the command line, a second launch or the file manager. Like the
/// original, audio and playlists replace the playlist and play; a `.wsz` loads as skin.
fn open_paths(app: &tauri::AppHandle, paths: Vec<PathBuf>) {
    let (skins, files): (Vec<_>, Vec<_>) = paths
        .into_iter()
        .partition(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("wsz")));
    if let Some(skin) = skins.into_iter().last() {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            if let Err(e) = skins::load_skin(Some(skin), app).await {
                log::warn!("cannot load skin: {e}");
            }
        });
    }
    if !files.is_empty() {
        player::pl_open(files, app.state::<Player>(), app.clone());
    }
}

/// Existing files among command-line arguments, relative ones resolved against `cwd`.
fn file_args<A: AsRef<Path>>(args: impl IntoIterator<Item = A>, cwd: &Path) -> Vec<PathBuf> {
    args.into_iter().map(|a| cwd.join(a)).filter(|p| p.exists()).collect()
}

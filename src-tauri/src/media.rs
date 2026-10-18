//! System media controls: MPRIS on Linux (media keys, desktop applets), SMTC on
//! Windows and Now Playing on macOS, through `souvlaki`.

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use audio_engine::PlayerState;
use souvlaki::{
    MediaControlEvent, MediaControls, MediaMetadata, MediaPlayback, MediaPosition, PlatformConfig,
    SeekDirection,
};
use tauri::{AppHandle, Manager};

use crate::player::{self, Player};

const SEEK_STEP: f64 = 5.0;

#[derive(Default)]
pub struct Media(Mutex<Option<Controls>>);

struct Controls {
    controls: MediaControls,
    /// Last published (title, artist, duration); metadata signals only go out on change.
    published: Option<(Option<String>, Option<String>, Option<Duration>)>,
}

// SAFETY: the Windows backend is not marked Send because it holds WinRT handles and a
// raw HWND; those are agile/thread-independent, and access is serialized by the mutex.
#[cfg(target_os = "windows")]
unsafe impl Send for Media {}
#[cfg(target_os = "windows")]
unsafe impl Sync for Media {}

/// Register with the desktop. Failure (no session bus, for instance) only disables the feature.
pub fn start(app: &AppHandle) {
    #[cfg(target_os = "windows")]
    let hwnd = app.get_webview_window("main").and_then(|w| w.hwnd().ok()).map(|h| h.0 as _);
    #[cfg(not(target_os = "windows"))]
    let hwnd = None;

    let config = PlatformConfig { display_name: "Kyara Player", dbus_name: "kyaraplayer", hwnd };
    let mut controls = match MediaControls::new(config) {
        Ok(c) => c,
        Err(e) => {
            log::warn!("media controls unavailable: {e:?}");
            return;
        }
    };
    let handle = app.clone();
    if let Err(e) = controls.attach(move |event| on_event(&handle, event)) {
        log::warn!("media controls: cannot attach: {e:?}");
        return;
    }
    *app.state::<Media>().0.lock().unwrap() = Some(Controls { controls, published: None });
    refresh(app);

    // The MPRIS Position property is a stored value: keep it current while playing.
    // This also picks up seeks, which the engine applies asynchronously.
    let handle = app.clone();
    let spawned = std::thread::Builder::new().name("media-position".into()).spawn(move || loop {
        std::thread::sleep(Duration::from_secs(1));
        if handle.state::<Player>().engine.status().state == PlayerState::Playing {
            refresh(&handle);
        }
    });
    if let Err(e) = spawned {
        log::warn!("media controls: no position updates: {e}");
    }
}

fn on_event(app: &AppHandle, event: MediaControlEvent) {
    let player = app.state::<Player>();
    let engine = &player.engine;
    let status = engine.status();
    let seek_by = |delta: f64| {
        if status.state != PlayerState::Stopped {
            engine.seek((status.position + delta).max(0.0));
        }
    };
    match event {
        MediaControlEvent::Play => player::play_pressed(player.clone(), app.clone()),
        MediaControlEvent::Pause => engine.pause(),
        MediaControlEvent::Toggle if status.state == PlayerState::Stopped => {
            player::pl_play(None, player.clone(), app.clone())
        }
        MediaControlEvent::Toggle => engine.toggle_pause(),
        MediaControlEvent::Next => player::pl_next(player.clone(), app.clone()),
        MediaControlEvent::Previous => player::pl_prev(player.clone(), app.clone()),
        MediaControlEvent::Stop => engine.stop(),
        MediaControlEvent::Seek(dir) => seek_by(signed(dir, SEEK_STEP)),
        MediaControlEvent::SeekBy(dir, d) => seek_by(signed(dir, d.as_secs_f64())),
        MediaControlEvent::SetPosition(MediaPosition(p)) => engine.seek(p.as_secs_f64()),
        MediaControlEvent::OpenUri(uri) if audio_engine::stream::is_url(&uri) => {
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = player::pl_add_url(uri, true, app).await {
                    log::warn!("open uri: {e}");
                }
            });
        }
        MediaControlEvent::OpenUri(uri) => {
            let path = uri.strip_prefix("file://").map(PathBuf::from).unwrap_or_else(|| uri.into());
            player::pl_open(vec![path], player.clone(), app.clone());
        }
        MediaControlEvent::Raise => {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.unminimize();
                let _ = w.show();
                let _ = w.set_focus();
            }
        }
        MediaControlEvent::Quit => app.exit(0),
        // The volume is the player's own; desktop applets may only read it.
        MediaControlEvent::SetVolume(_) => {}
    }
    // Seeks do not produce engine events.
    refresh(app);
}

fn signed(dir: SeekDirection, secs: f64) -> f64 {
    match dir {
        SeekDirection::Forward => secs,
        SeekDirection::Backward => -secs,
    }
}

/// Publish the current track and playback state.
pub fn refresh(app: &AppHandle) {
    let media = app.state::<Media>();
    let mut guard = media.0.lock().unwrap();
    let Some(Controls { controls, published }) = guard.as_mut() else { return };
    let player = app.state::<Player>();
    let status = player.engine.status();
    let title = player.list.lock().unwrap().current().map(|e| e.title.clone());
    // Classic titles read "Artist - Title".
    let (artist, title) = match title.as_deref().and_then(|t| t.split_once(" - ")) {
        Some((a, t)) => (Some(a.to_string()), Some(t.to_string())),
        None => (None, title),
    };
    let duration = status.track.as_ref().and_then(|t| t.duration).map(Duration::from_secs_f64);
    let meta = (title, artist, duration);
    if published.as_ref() != Some(&meta) {
        let _ = controls.set_metadata(MediaMetadata {
            title: meta.0.as_deref(),
            artist: meta.1.as_deref(),
            duration: meta.2,
            ..Default::default()
        });
        *published = Some(meta);
    }
    let progress = Some(MediaPosition(Duration::from_secs_f64(status.position.max(0.0))));
    let _ = controls.set_playback(match status.state {
        PlayerState::Playing => MediaPlayback::Playing { progress },
        PlayerState::Paused => MediaPlayback::Paused { progress },
        PlayerState::Stopped => MediaPlayback::Stopped,
    });
}

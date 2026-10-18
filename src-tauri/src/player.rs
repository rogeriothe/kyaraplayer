//! Playback engine + playlist: commands, auto-advance, tag loading and persistence.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use audio_engine::stream::{self, Remote};
use audio_engine::{Engine, Event, Status, VisFrame};
use crossbeam_channel::{unbounded, Receiver, RecvTimeoutError, Sender};
use playlist::{formats, meta, EntryId, Playlist, SortKey};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::{config, media, tray};

pub struct Player {
    pub engine: Engine,
    pub list: Mutex<Playlist>,
    meta_tx: Sender<(EntryId, PathBuf)>,
    /// Playlist changed since it was last written to disk.
    dirty: AtomicBool,
}

#[derive(Serialize, Clone)]
pub struct EntryView {
    id: EntryId,
    title: String,
    duration: Option<f64>,
}

#[derive(Serialize, Clone)]
pub struct PlaylistView {
    entries: Vec<EntryView>,
    current: Option<EntryId>,
    shuffle: bool,
    repeat: bool,
    /// Sum of known durations in seconds.
    total: f64,
    /// Every entry's duration is known.
    total_complete: bool,
}

impl Player {
    /// Create the player. Call [`Player::start`] once the app handle exists.
    pub fn new() -> (Self, Receiver<(EntryId, PathBuf)>) {
        let (meta_tx, meta_rx) = unbounded();
        let player = Self {
            engine: Engine::new(),
            list: Mutex::new(Playlist::new()),
            meta_tx,
            dirty: AtomicBool::new(false),
        };
        (player, meta_rx)
    }

    fn view(&self) -> PlaylistView {
        let list = self.list.lock().unwrap();
        let (total, total_complete) = list.total_duration();
        PlaylistView {
            entries: list
                .entries()
                .iter()
                .map(|e| EntryView { id: e.id, title: e.title.clone(), duration: e.duration })
                .collect(),
            current: list.current_id(),
            shuffle: list.shuffle(),
            repeat: list.repeat(),
            total,
            total_complete,
        }
    }

    fn queue_meta(&self, ids: &[EntryId]) {
        let list = self.list.lock().unwrap();
        for id in ids {
            // Streams have no tags to read; their titles come from the server.
            if let Some(e) = list.get(*id).filter(|e| !e.path.to_str().is_some_and(stream::is_url)) {
                let _ = self.meta_tx.send((e.id, e.path.clone()));
            }
        }
    }
}

fn playlist_file(app: &AppHandle) -> Option<PathBuf> {
    config::dir(app).map(|d| d.join("playlist.m3u8"))
}

/// Notify every window and mark the list for saving.
fn changed(app: &AppHandle) {
    let player = app.state::<Player>();
    player.dirty.store(true, Ordering::Relaxed);
    let _ = app.emit("playlist", player.view());
    media::refresh(app);
    let title = player.list.lock().unwrap().current().map(|e| e.title.clone());
    tray::set_title(app, title.as_deref());
}

/// Play the playlist's current entry.
fn play_current(app: &AppHandle) {
    let player = app.state::<Player>();
    let path = player.list.lock().unwrap().current().map(|e| e.path.clone());
    if let Some(path) = path {
        player.engine.play(path);
    }
    changed(app);
}

/// Background work: restore the saved playlist, read tags, save on change,
/// and advance when a track ends.
pub fn start(app: &AppHandle, meta_rx: Receiver<(EntryId, PathBuf)>) {
    let player = app.state::<Player>();
    let cfg = config::load(app);
    player.engine.set_volume(cfg.volume);
    player.engine.set_balance(cfg.balance);
    {
        let mut list = player.list.lock().unwrap();
        if let Some(file) = playlist_file(app).filter(|f| f.exists()) {
            list.add_paths(&[file], None);
        }
        list.set_repeat(cfg.repeat);
        if let Some(id) = cfg.playlist_current.and_then(|i| list.entries().get(i)).map(|e| e.id) {
            list.set_current(id);
        }
        list.set_shuffle(cfg.shuffle);
    }
    let ids: Vec<EntryId> = player.list.lock().unwrap().entries().iter().map(|e| e.id).collect();
    player.queue_meta(&ids);

    spawn("playlist-meta", {
        let app = app.clone();
        move || meta_worker(app, meta_rx)
    });
    spawn("playlist-save", {
        let app = app.clone();
        move || save_worker(app)
    });
    spawn("engine-events", {
        let app = app.clone();
        let events = player.engine.events();
        move || {
            for event in events {
                if let Event::StreamTitle { title } = &event {
                    let player = app.state::<Player>();
                    let updated = {
                        let mut list = player.list.lock().unwrap();
                        let current = list.current_id();
                        current.and_then(|id| list.get_mut(id)).map(|e| e.title = title.clone()).is_some()
                    };
                    if updated {
                        changed(&app);
                    }
                }
                if matches!(event, Event::TrackEnded) {
                    let next = app.state::<Player>().list.lock().unwrap().next(true).is_some();
                    if next {
                        play_current(&app);
                    }
                }
                if let Err(e) = app.emit("engine", &event) {
                    log::warn!("emit engine event: {e}");
                }
                if let Event::TrackStarted { track } = &event {
                    crate::eq::on_track_started(&app, &app.state::<Player>(), &track.path);
                }
                if matches!(event, Event::StateChanged { .. } | Event::TrackStarted { .. }) {
                    media::refresh(&app);
                }
            }
        }
    });
}

fn spawn(name: &str, f: impl FnOnce() + Send + 'static) {
    if let Err(e) = std::thread::Builder::new().name(name.into()).spawn(f) {
        log::error!("cannot start {name} thread: {e}");
    }
}

/// Read tags off the UI thread, publishing updates at most every 250 ms.
fn meta_worker(app: AppHandle, rx: Receiver<(EntryId, PathBuf)>) {
    let mut pending = false;
    let mut last_emit = Instant::now();
    loop {
        match rx.recv_timeout(Duration::from_millis(250)) {
            Ok((id, path)) => {
                let meta = meta::read(&path);
                let player = app.state::<Player>();
                let mut list = player.list.lock().unwrap();
                if let Some(e) = list.get_mut(id) {
                    if let Some(m) = &meta {
                        if let Some(t) = m.display_title() {
                            e.title = t;
                        }
                        e.duration = m.duration.or(e.duration);
                    }
                    e.meta_loaded = true;
                    pending = true;
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        if pending && (rx.is_empty() || last_emit.elapsed() > Duration::from_millis(250)) {
            changed(&app);
            pending = false;
            last_emit = Instant::now();
        }
    }
}

/// Persist the playlist (and the current index) shortly after changes.
fn save_worker(app: AppHandle) {
    loop {
        std::thread::sleep(Duration::from_secs(1));
        let player = app.state::<Player>();
        if !player.dirty.swap(false, Ordering::Relaxed) {
            continue;
        }
        let (entries, current, shuffle, repeat) = {
            let list = player.list.lock().unwrap();
            let current = list.current_id().and_then(|id| list.index_of(id));
            (list.entries().to_vec(), current, list.shuffle(), list.repeat())
        };
        if let Some(file) = playlist_file(&app) {
            let _ = file.parent().map(std::fs::create_dir_all);
            if let Err(e) = formats::write_m3u8(&file, &entries) {
                log::warn!("cannot save playlist: {e}");
            }
        }
        config::update(&app, |c| {
            c.playlist_current = current;
            c.shuffle = shuffle;
            c.repeat = repeat;
        });
    }
}

// ---------------------------------------------------------------- engine commands

#[tauri::command]
pub fn toggle_pause(player: State<'_, Player>) {
    player.engine.toggle_pause();
}

#[tauri::command]
pub fn stop(player: State<'_, Player>) {
    player.engine.stop();
}

#[tauri::command]
pub fn seek(secs: f64, player: State<'_, Player>, app: AppHandle) {
    player.engine.seek(secs);
    media::refresh(&app);
}

#[tauri::command]
pub fn set_volume(volume: f32, player: State<'_, Player>, app: AppHandle) {
    player.engine.set_volume(volume);
    config::update(&app, |c| c.volume = volume.clamp(0.0, 1.0));
}

#[tauri::command]
pub fn set_balance(balance: f32, player: State<'_, Player>, app: AppHandle) {
    player.engine.set_balance(balance);
    config::update(&app, |c| c.balance = balance.clamp(-1.0, 1.0));
}

#[tauri::command]
pub fn status(player: State<'_, Player>) -> Status {
    player.engine.status()
}

#[tauri::command]
pub fn vis_frame(player: State<'_, Player>) -> VisFrame {
    player.engine.vis_frame()
}

// ---------------------------------------------------------------- playlist commands

#[tauri::command]
pub fn pl_view(player: State<'_, Player>) -> PlaylistView {
    player.view()
}

/// Add files, folders or playlists (at `at`, or at the end).
#[tauri::command]
pub fn pl_add(paths: Vec<PathBuf>, at: Option<usize>, player: State<'_, Player>, app: AppHandle) {
    let ids = player.list.lock().unwrap().add_paths(&paths, at);
    player.queue_meta(&ids);
    changed(&app);
}

/// Open the file info window for entry `id`, or for the current entry.
#[tauri::command]
pub fn pl_file_info(id: Option<EntryId>, player: State<'_, Player>, app: AppHandle) -> Result<(), String> {
    let path = {
        let list = player.list.lock().unwrap();
        let entry = match id {
            Some(id) => list.get(id),
            None => list.current().or(list.entries().first()),
        };
        entry.map(|e| e.path.to_string_lossy().into_owned()).ok_or("playlist is empty")?
    };
    crate::windows::open_file_info(path, app)
}

/// Tags and stream properties for the file info window.
#[tauri::command]
pub async fn file_info(path: PathBuf) -> Result<meta::Details, String> {
    if path.to_str().is_some_and(stream::is_url) {
        return Err("streams have no file information".into());
    }
    tauri::async_runtime::spawn_blocking(move || meta::details(&path)).await.map_err(|e| e.to_string())?
}

/// Write tags, then refresh the titles of playlist entries for that file.
#[tauri::command]
pub async fn file_info_save(path: PathBuf, tags: meta::Tags, app: AppHandle) -> Result<(), String> {
    let p = path.clone();
    tauri::async_runtime::spawn_blocking(move || meta::write_tags(&p, &tags)).await.map_err(|e| e.to_string())??;
    let player = app.state::<Player>();
    let ids: Vec<EntryId> =
        player.list.lock().unwrap().entries().iter().filter(|e| e.path == path).map(|e| e.id).collect();
    player.queue_meta(&ids);
    Ok(())
}

/// Add an `http(s)://` address: a stream, or a remote `.pls`/`.m3u` whose entries are
/// added instead. With `play`, replaces the playlist and starts playing ("Play URL").
#[tauri::command]
pub async fn pl_add_url(url: String, play: bool, app: AppHandle) -> Result<usize, String> {
    let url = url.trim().to_string();
    if !stream::is_url(&url) {
        return Err("use an http:// or https:// address".into());
    }
    let resolved = {
        let url = url.clone();
        tauri::async_runtime::spawn_blocking(move || stream::resolve(&url))
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string())?
    };
    let items = match resolved {
        Remote::Audio => vec![formats::Item { path: PathBuf::from(&url), title: None, duration: None }],
        Remote::Playlist(text) => {
            let items = if text.to_ascii_lowercase().contains("[playlist]") {
                formats::parse_pls(&text, Path::new(""))
            } else {
                formats::parse_m3u(&text, Path::new(""))
            };
            // Relative entries mean nothing without the remote folder; keep addresses.
            let items: Vec<_> =
                items.into_iter().filter(|i| i.path.to_str().is_some_and(stream::is_url)).collect();
            if items.is_empty() {
                return Err("the playlist has no stream addresses".into());
            }
            items
        }
    };
    let count = items.len();
    let player = app.state::<Player>();
    {
        let mut list = player.list.lock().unwrap();
        if play {
            list.clear();
        }
        let ids = list.add_items(items, None);
        if let (true, Some(first)) = (play, ids.first()) {
            list.set_current(*first);
        }
    }
    if play {
        play_current(&app);
    } else {
        changed(&app);
    }
    Ok(count)
}

/// Replace the playlist with `paths` and play the first one ("Play file" in the original).
#[tauri::command]
pub fn pl_open(paths: Vec<PathBuf>, player: State<'_, Player>, app: AppHandle) {
    let ids = {
        let mut list = player.list.lock().unwrap();
        list.clear();
        let ids = list.add_paths(&paths, None);
        if let Some(first) = ids.first() {
            list.set_current(*first);
        }
        ids
    };
    player.queue_meta(&ids);
    play_current(&app);
}

#[tauri::command]
pub fn pl_remove(ids: Vec<EntryId>, player: State<'_, Player>, app: AppHandle) {
    player.list.lock().unwrap().remove(&ids);
    changed(&app);
}

#[tauri::command]
pub fn pl_crop(ids: Vec<EntryId>, player: State<'_, Player>, app: AppHandle) {
    player.list.lock().unwrap().crop(&ids);
    changed(&app);
}

#[tauri::command]
pub fn pl_clear(player: State<'_, Player>, app: AppHandle) {
    player.list.lock().unwrap().clear();
    changed(&app);
}

/// Remove missing files (`"dead"`) or repeated paths (`"duplicates"`).
#[tauri::command]
pub fn pl_remove_misc(kind: String, player: State<'_, Player>, app: AppHandle) -> usize {
    let mut list = player.list.lock().unwrap();
    let n = match kind.as_str() {
        "dead" => list.remove_dead(),
        "duplicates" => list.remove_duplicates(),
        _ => 0,
    };
    drop(list);
    changed(&app);
    n
}

#[tauri::command]
pub fn pl_move(ids: Vec<EntryId>, to: usize, player: State<'_, Player>, app: AppHandle) {
    player.list.lock().unwrap().move_to(&ids, to);
    changed(&app);
}

/// `title`, `filename`, `path`, `reverse` or `random`.
#[tauri::command]
pub fn pl_sort(kind: String, player: State<'_, Player>, app: AppHandle) {
    let mut list = player.list.lock().unwrap();
    match kind.as_str() {
        "title" => list.sort(SortKey::Title),
        "filename" => list.sort(SortKey::FileName),
        "path" => list.sort(SortKey::Path),
        "reverse" => list.reverse(),
        "random" => list.randomize(),
        _ => {}
    }
    drop(list);
    changed(&app);
}

/// Play button: resume when paused, otherwise (re)start the current entry.
pub fn play_pressed(player: State<'_, Player>, app: AppHandle) {
    if player.engine.status().state == audio_engine::PlayerState::Paused {
        player.engine.resume();
    } else {
        pl_play(None, player, app);
    }
}

/// Play `id`, or the current entry (the first one if none) when `id` is `None`.
#[tauri::command]
pub fn pl_play(id: Option<EntryId>, player: State<'_, Player>, app: AppHandle) {
    {
        let mut list = player.list.lock().unwrap();
        let target = id.or(list.current_id()).or(list.entries().first().map(|e| e.id));
        match target {
            Some(t) => {
                list.set_current(t);
            }
            None => return,
        }
    }
    play_current(&app);
}

#[tauri::command]
pub fn pl_next(player: State<'_, Player>, app: AppHandle) {
    if player.list.lock().unwrap().next(false).is_some() {
        play_current(&app);
    }
}

#[tauri::command]
pub fn pl_prev(player: State<'_, Player>, app: AppHandle) {
    if player.list.lock().unwrap().prev().is_some() {
        play_current(&app);
    }
}

#[tauri::command]
pub fn pl_jump(delta: isize, player: State<'_, Player>, app: AppHandle) {
    if player.list.lock().unwrap().jump(delta).is_some() {
        play_current(&app);
    }
}

#[tauri::command]
pub fn pl_set_shuffle(on: bool, player: State<'_, Player>, app: AppHandle) {
    player.list.lock().unwrap().set_shuffle(on);
    changed(&app);
}

#[tauri::command]
pub fn pl_set_repeat(on: bool, player: State<'_, Player>, app: AppHandle) {
    player.list.lock().unwrap().set_repeat(on);
    changed(&app);
}

/// Save as `.pls` or (default) `.m3u8`.
#[tauri::command]
pub fn pl_save(path: PathBuf, player: State<'_, Player>) -> Result<(), String> {
    let entries = player.list.lock().unwrap().entries().to_vec();
    let pls = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("pls"));
    let result = if pls { formats::write_pls(&path, &entries) } else { formats::write_m3u8(&path, &entries) };
    result.map_err(|e| e.to_string())
}

/// Replace the playlist with a playlist file.
#[tauri::command]
pub fn pl_load(path: PathBuf, player: State<'_, Player>, app: AppHandle) {
    let ids = {
        let mut list = player.list.lock().unwrap();
        list.clear();
        list.add_paths(&[path], None)
    };
    player.queue_meta(&ids);
    changed(&app);
}

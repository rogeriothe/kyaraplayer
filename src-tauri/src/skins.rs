//! Active skin state, the `skin://` protocol and skin commands.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

use serde::Serialize;
use skin::{Skin, SkinInfo};
use tauri::http::{header, Response, StatusCode};
use tauri::{AppHandle, Emitter, Manager, State};

use crate::config;

pub struct SkinState {
    current: RwLock<Arc<Skin>>,
    /// Bumped on every skin change so the UI can bust cached sheet URLs.
    generation: AtomicU64,
}

impl Default for SkinState {
    fn default() -> Self {
        Self { current: RwLock::new(Arc::new(Skin::default_skin())), generation: AtomicU64::new(0) }
    }
}

impl SkinState {
    fn get(&self) -> Arc<Skin> {
        self.current.read().unwrap().clone()
    }

    pub fn set(&self, skin: Skin) -> SkinPayload {
        *self.current.write().unwrap() = Arc::new(skin);
        self.generation.fetch_add(1, Ordering::Relaxed);
        self.payload()
    }

    fn payload(&self) -> SkinPayload {
        SkinPayload { info: self.get().info(), generation: self.generation.load(Ordering::Relaxed) }
    }
}

#[derive(Clone, Serialize)]
pub struct SkinPayload {
    #[serde(flatten)]
    info: SkinInfo,
    generation: u64,
}

#[tauri::command]
pub fn skin_info(skins: State<'_, SkinState>) -> SkinPayload {
    skins.payload()
}

/// Load a `.wsz` skin (or skin folder); `None` restores the built-in skin.
#[tauri::command]
pub async fn load_skin(path: Option<PathBuf>, app: AppHandle) -> Result<SkinPayload, String> {
    let skin = match &path {
        Some(p) => {
            let p = p.clone();
            tauri::async_runtime::spawn_blocking(move || Skin::load(&p))
                .await
                .map_err(|e| e.to_string())?
                .map_err(|e| e.to_string())?
        }
        None => Skin::default_skin(),
    };
    let payload = app.state::<SkinState>().set(skin);
    config::update(&app, |c| c.skin = path);
    let _ = app.emit("skin-changed", &payload);
    Ok(payload)
}

/// Restore the skin saved in the config, if it still loads.
pub fn restore(app: &AppHandle) {
    if let Some(path) = config::load(app).skin {
        match Skin::load(&path) {
            Ok(skin) => {
                app.state::<SkinState>().set(skin);
            }
            Err(e) => log::warn!("cannot restore skin {}: {e}", path.display()),
        }
    }
}

/// Serves skin sheets at `skin://localhost/<sheet>.png` and cursors at
/// `skin://localhost/cursor/<name>.png`.
pub fn protocol(app: &AppHandle, path: &str) -> Response<Vec<u8>> {
    let skin = app.state::<SkinState>().get();
    let name = path.trim_start_matches('/');
    let png = match name.strip_prefix("cursor/") {
        Some(cursor) => skin.cursor(cursor.trim_end_matches(".png")),
        None => skin.sheet(name),
    };
    match png {
        Some(png) => Response::builder()
            .header(header::CONTENT_TYPE, "image/png")
            .header(header::CACHE_CONTROL, "no-cache")
            // Lets the UI crop sheets on a canvas without tainting it.
            .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
            .body(png.to_vec()),
        None => Response::builder().status(StatusCode::NOT_FOUND).body(Vec::new()),
    }
    .expect("valid response")
}

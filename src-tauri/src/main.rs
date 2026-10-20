#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tauri::Manager;
use waw_image_viewer::catalog::{Catalog, Index};

type SharedCatalog = Arc<Mutex<Option<Catalog>>>;

fn game_root() -> Result<PathBuf, String> {
    let root = if cfg!(debug_assertions) {
        std::env::var_os("WAW_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.local/test-root")
            })
    } else {
        std::env::current_exe()
            .map_err(|e| e.to_string())?
            .parent()
            .ok_or("Cannot locate the app folder.")?
            .to_path_buf()
    };
    if !root.join("CoDWaW.exe").is_file() {
        return Err("Put this executable in your World at War folder, beside CoDWaW.exe, then open it again.".into());
    }
    Ok(root)
}

#[tauri::command]
fn startup(app: tauri::AppHandle, state: tauri::State<SharedCatalog>) -> Result<(), String> {
    let root = game_root()?;
    let cache = app
        .path()
        .app_cache_dir()
        .map_err(|e| e.to_string())?
        .join("iwi-v1");
    *state.lock().map_err(|e| e.to_string())? = Some(Catalog::new(root, cache));
    Ok(())
}

// Disk access and decoding stay off the WebView's event thread. The mutex also
// serializes cache changes with thumbnails, previews and exports.
async fn with_catalog<T: Send + 'static>(
    state: SharedCatalog,
    work: impl FnOnce(&mut Catalog) -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut guard = state.lock().map_err(|e| e.to_string())?;
        work(
            guard
                .as_mut()
                .ok_or("Open the app beside CoDWaW.exe first.")?,
        )
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn scan(force: bool, state: tauri::State<'_, SharedCatalog>) -> Result<Index, String> {
    with_catalog(state.inner().clone(), move |catalog| catalog.scan(force)).await
}

#[tauri::command]
async fn thumbnail(id: String, state: tauri::State<'_, SharedCatalog>) -> Result<String, String> {
    with_catalog(state.inner().clone(), move |catalog| catalog.thumbnail(&id)).await
}

#[tauri::command]
async fn pixels(
    id: String,
    face: usize,
    state: tauri::State<'_, SharedCatalog>,
) -> Result<tauri::ipc::Response, String> {
    let bytes = with_catalog(state.inner().clone(), move |catalog| {
        catalog.pixels(&id, face)
    })
    .await?;
    Ok(tauri::ipc::Response::new(bytes))
}

#[tauri::command]
async fn export_conflicts(
    id: String,
    destination: String,
    kind: String,
    state: tauri::State<'_, SharedCatalog>,
) -> Result<Vec<String>, String> {
    with_catalog(state.inner().clone(), move |catalog| {
        Ok(catalog
            .export_paths(&id, &PathBuf::from(destination), &kind)?
            .iter()
            .filter(|path| path.exists())
            .map(|path| path.display().to_string())
            .collect())
    })
    .await
}

#[tauri::command]
async fn export_image(
    id: String,
    destination: String,
    kind: String,
    replace: bool,
    state: tauri::State<'_, SharedCatalog>,
) -> Result<Vec<String>, String> {
    with_catalog(state.inner().clone(), move |catalog| {
        catalog.export(&id, &PathBuf::from(destination), &kind, replace)
    })
    .await
}

#[tauri::command]
async fn clear_cache(state: tauri::State<'_, SharedCatalog>) -> Result<(), String> {
    with_catalog(state.inner().clone(), |catalog| catalog.clear_cache()).await
}

#[tauri::command]
async fn open_cache(state: tauri::State<'_, SharedCatalog>) -> Result<(), String> {
    with_catalog(state.inner().clone(), |catalog| {
        std::fs::create_dir_all(&catalog.cache).map_err(|e| e.to_string())?;
        std::process::Command::new("explorer.exe")
            .arg(&catalog.cache)
            .spawn()
            .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

#[tauri::command]
fn quit(app: tauri::AppHandle) {
    app.exit(0);
}

fn main() {
    let profile = tempfile::Builder::new()
        .prefix("waw-image-ui-")
        .tempdir()
        .expect("Cannot create the temporary browser profile");
    let profile_path = profile.path().to_path_buf();
    tauri::Builder::default()
        .manage(Arc::new(Mutex::new(None::<Catalog>)))
        .plugin(tauri_plugin_dialog::init())
        .setup(move |app| {
            tauri::WebviewWindowBuilder::from_config(app, &app.config().app.windows[0])?
                .data_directory(profile_path)
                .build()?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            startup,
            scan,
            thumbnail,
            pixels,
            export_conflicts,
            export_image,
            clear_cache,
            open_cache,
            quit
        ])
        .run(tauri::generate_context!())
        .expect("Cannot start WaW Image Viewer");
}

pub mod commands;
pub mod downloader;
pub mod metadata;

#[cfg(not(target_os = "android"))]
pub mod dependencies;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use tauri::Manager;

pub struct AppState {
    pub settings: std::sync::Mutex<commands::Settings>,
    pub settings_path: PathBuf,
    pub running: AtomicBool,
    pub pending_deletions: std::sync::Mutex<HashMap<String, String>>,
    pub pending_conflicts: std::sync::Mutex<HashMap<String, String>>,
    pub conflict_matches: std::sync::Mutex<HashMap<String, Vec<String>>>,
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let settings_path = app.path().app_config_dir()?.join("settings.json");
            let settings = commands::resolve_settings(&settings_path);
            let state = AppState {
                settings: std::sync::Mutex::new(settings),
                settings_path,
                running: AtomicBool::new(false),
                pending_deletions: std::sync::Mutex::new(HashMap::new()),
                pending_conflicts: std::sync::Mutex::new(HashMap::new()),
                conflict_matches: std::sync::Mutex::new(HashMap::new()),
            };
            app.manage(state);

            #[cfg(not(target_os = "android"))]
            {
                let app_handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    dependencies::ensure_dependencies(app_handle).await;
                });
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::search_tracks,
            commands::resolve_input,
            commands::download_track,
            commands::pick_folder,
            commands::get_settings,
            commands::save_settings,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

pub mod commands;
pub mod downloader;
pub mod jobs;
pub mod metadata;
pub mod naming;
pub mod sync;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use tauri::Manager;

pub struct JobControl {
    pub job_id: String,
    pub cancel: std::sync::Arc<AtomicBool>,
}

#[derive(Debug, Clone)]
pub enum ConflictResolution {
    SyncHere(PathBuf),
    CreateNew,
    Cancel,
}

pub struct AppState {
    pub settings: std::sync::Mutex<commands::Settings>,
    pub settings_path: PathBuf,
    pub running: AtomicBool,
    pub job: std::sync::Mutex<Option<JobControl>>,
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
                job: std::sync::Mutex::new(None),
                pending_deletions: std::sync::Mutex::new(HashMap::new()),
                pending_conflicts: std::sync::Mutex::new(HashMap::new()),
                conflict_matches: std::sync::Mutex::new(HashMap::new()),
            };
            app.manage(state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::search_tracks,
            commands::resolve_spotify_link,
            commands::download_track,
            commands::pick_folder,
            commands::get_settings,
            commands::save_settings,
            commands::start_job,
            commands::cancel_job,
            commands::confirm_deletions,
            commands::resolve_conflict,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

pub mod auth;
pub mod commands;
pub mod downloader;
pub mod jobs;
pub mod metadata;
pub mod naming;
pub mod sync;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
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
    pub auth: tokio::sync::Mutex<Option<rspotify::AuthCodePkceSpotify>>,
    pub auth_cancel: std::sync::Arc<AtomicBool>,
    pub auth_waiting: AtomicBool,
    pub display_name: Mutex<Option<String>>,
    pub settings: Mutex<auth::Settings>,
    pub settings_path: PathBuf,
    pub running: AtomicBool,
    pub job: Mutex<Option<JobControl>>,
    pub pending_deletions: Mutex<HashMap<String, tokio::sync::oneshot::Sender<String>>>,
    pub pending_conflicts: Mutex<HashMap<String, tokio::sync::oneshot::Sender<ConflictResolution>>>,
    pub conflict_matches: Mutex<HashMap<String, Vec<PathBuf>>>,
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            auth::load_env_files();

            let settings_path = app.path().app_config_dir()?.join("settings.json");
            let settings = auth::resolve_settings(&settings_path);

            let restored = match (&settings.client_id, auth::load_token()) {
                (Some(client_id), Some(token)) => Some(auth::restore_client(
                    client_id,
                    &settings.redirect_uri,
                    token,
                )),
                _ => None,
            };

            let state = AppState {
                auth: tokio::sync::Mutex::new(restored),
                auth_cancel: std::sync::Arc::new(AtomicBool::new(false)),
                auth_waiting: AtomicBool::new(false),
                display_name: Mutex::new(None),
                settings: Mutex::new(settings),
                settings_path,
                running: AtomicBool::new(false),
                job: Mutex::new(None),
                pending_deletions: Mutex::new(HashMap::new()),
                pending_conflicts: Mutex::new(HashMap::new()),
                conflict_matches: Mutex::new(HashMap::new()),
            };

            app.manage(state);

            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                jobs::restore_session(handle).await;
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::pick_folder,
            commands::auth_status,
            commands::auth_connect,
            commands::auth_cancel,
            commands::auth_logout,
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

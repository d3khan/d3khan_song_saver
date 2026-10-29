pub mod batch;
pub mod commands;
pub mod downloader;
pub mod metadata;

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use tauri::{
    Manager, WindowEvent,
    menu::{MenuBuilder, MenuItemBuilder},
    tray::TrayIconBuilder,
};

pub struct AppState {
    pub settings: std::sync::Mutex<commands::Settings>,
    pub settings_path: PathBuf,
    pub running: AtomicBool,
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_shell::init())
        .setup(|app| {
            let settings_path = app.path().app_config_dir()?.join("settings.json");
            let settings = commands::resolve_settings(&settings_path);
            app.manage(AppState {
                settings: std::sync::Mutex::new(settings),
                settings_path,
                running: AtomicBool::new(false),
            });
            app.manage(batch::BatchControl::default());

            // Create the System Tray
            let quit = MenuItemBuilder::with_id("quit", "Quit").build(app)?;
            let show = MenuItemBuilder::with_id("show", "Show").build(app)?;
            let menu = MenuBuilder::new(app).items(&[&show, &quit]).build()?;

            let _tray = TrayIconBuilder::new()
                .menu(&menu)
                .icon(app.default_window_icon().unwrap().clone()) // <-- ADDED ICON HERE
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "quit" => app.exit(0),
                    "show" => {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.unminimize();
                            let _ = window.set_focus();
                        }
                    }
                    _ => {}
                })
                .build(app)?;

            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                window.hide().unwrap();
                api.prevent_close();
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::resolve_input,
            commands::resolve_bulk_input,
            commands::download_track,
            commands::pick_folder,
            batch::resume_batch,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

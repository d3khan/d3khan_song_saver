pub mod batch;
pub mod commands;
pub mod downloader;
pub mod metadata;

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use tauri::{
    menu::{MenuBuilder, MenuItemBuilder},
    tray::TrayIconBuilder,
    Manager, WindowEvent,
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
            // --- EXPIRY GUARD (Jan 1, 2027) ---
            const EXPIRY_UNIX: u64 = 1_798_761_600;
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);

            if now >= EXPIRY_UNIX {
                use tauri_plugin_dialog::{DialogExt, MessageDialogKind, MessageDialogButtons};
                let should_update = app.dialog()
                    .message("This version of d3khan song saver has expired.\n\nClick 'Update' to go to the latest release.")
                    .title("Version Expired")
                    .kind(MessageDialogKind::Error)
                    .buttons(MessageDialogButtons::OkCancelCustom("Update".into(), "Exit".into()))
                    .blocking_show();
                if should_update {
                    let url = "https://github.com/d3khan/d3khan_song_saver/releases";
                    #[cfg(target_os = "windows")]
                    let _ = std::process::Command::new("cmd").args(["/C", "start", url]).spawn();
                    #[cfg(target_os = "macos")]
                    let _ = std::process::Command::new("open").arg(url).spawn();
                    #[cfg(target_os = "linux")]
                    let _ = std::process::Command::new("xdg-open").arg(url).spawn();
                }
                std::process::exit(0);
            }
            // --- END EXPIRY GUARD ---
            
            let settings_path = app.path().app_config_dir()?.join("settings.json");
            let settings = commands::resolve_settings(&settings_path);
            app.manage(AppState {
                settings: std::sync::Mutex::new(settings),
                settings_path,
                running: AtomicBool::new(false),
            });
            app.manage(batch::BatchControl::default());

            // System Tray
            let quit = MenuItemBuilder::with_id("quit", "Quit").build(app)?;
            let show = MenuItemBuilder::with_id("show", "Show").build(app)?;
            let menu = MenuBuilder::new(app).items(&[&show, &quit]).build()?;
            let _tray = TrayIconBuilder::new()
                .menu(&menu)
                .icon(app.default_window_icon().unwrap().clone())
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
            commands::search_youtube,
            commands::probe_size,
            commands::download_track,
            commands::pick_folder,
            commands::get_settings,
            commands::save_settings,
            commands::delete_orphaned_files,
            batch::resume_batch,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}